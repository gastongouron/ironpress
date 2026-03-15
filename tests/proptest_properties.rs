use proptest::prelude::*;

/// Generate arbitrary HTML-like strings
fn arb_html() -> impl Strategy<Value = String> {
    prop::string::string_regex("<[a-z]{1,5}( [a-z]+=\"[^\"]*\")*>[^<]{0,100}</[a-z]{1,5}>").unwrap()
}

/// Generate arbitrary CSS-like strings
fn arb_css() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z]{1,10} \\{ [a-z-]{1,15}: [a-z0-9#%]{1,10}; \\}").unwrap()
}

proptest! {
    /// html_to_pdf should never panic on any valid UTF-8 input
    #[test]
    fn html_to_pdf_never_panics(s in "\\PC{0,500}") {
        let _ = ironpress::html_to_pdf(&s);
    }

    /// markdown_to_pdf should never panic on any valid UTF-8 input
    #[test]
    fn markdown_to_pdf_never_panics(s in "\\PC{0,500}") {
        let _ = ironpress::markdown_to_pdf(&s);
    }

    /// Output is always valid (non-empty) when input contains at least some content
    #[test]
    fn html_output_starts_with_pdf_header(s in "[a-zA-Z ]{1,100}") {
        let html = format!("<p>{}</p>", s);
        let result = ironpress::html_to_pdf(&html);
        prop_assert!(result.is_ok());
        let pdf = result.unwrap();
        prop_assert!(pdf.starts_with(b"%PDF-1.4"));
    }

    /// Generated PDFs always have at least one page
    #[test]
    fn pdf_always_has_pages(s in "[a-zA-Z0-9 .,!?]{1,200}") {
        let html = format!("<p>{}</p>", s);
        let pdf = ironpress::html_to_pdf(&html).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        prop_assert!(pdf_str.contains("/Type /Page"));
    }

    /// CSS in style tags should never cause a panic
    #[test]
    fn css_in_style_never_panics(css in arb_css()) {
        let html = format!("<style>{}</style><p>test</p>", css);
        let _ = ironpress::html_to_pdf(&html);
    }

    /// Generated HTML structures should not panic
    #[test]
    fn generated_html_never_panics(tag in arb_html()) {
        let _ = ironpress::html_to_pdf(&tag);
    }

    /// Markdown with various heading levels never panics
    #[test]
    fn markdown_headings_never_panic(level in 1u8..7, text in "[a-zA-Z ]{1,50}") {
        let hashes = "#".repeat(level as usize);
        let md = format!("{} {}\n\nSome body text.", hashes, text);
        let result = ironpress::markdown_to_pdf(&md);
        prop_assert!(result.is_ok());
    }

    /// Very deeply nested HTML should not stack overflow
    #[test]
    fn nested_html_no_stack_overflow(depth in 1usize..50) {
        let open: String = (0..depth).map(|_| "<div>").collect();
        let close: String = (0..depth).map(|_| "</div>").collect();
        let html = format!("{}text{}", open, close);
        let _ = ironpress::html_to_pdf(&html);
    }

    /// Tables with varying dimensions should not panic
    #[test]
    fn table_dimensions_never_panic(rows in 1usize..10, cols in 1usize..8) {
        let header: String = (0..cols).map(|c| format!("<th>H{}</th>", c)).collect();
        let body_rows: String = (0..rows).map(|r| {
            let cells: String = (0..cols).map(|c| format!("<td>R{}C{}</td>", r, c)).collect();
            format!("<tr>{}</tr>", cells)
        }).collect();
        let html = format!("<table><thead><tr>{}</tr></thead><tbody>{}</tbody></table>", header, body_rows);
        let result = ironpress::html_to_pdf(&html);
        prop_assert!(result.is_ok());
    }

    /// Font sizes should be handled without panic
    #[test]
    fn font_size_values_never_panic(size in 1u32..200) {
        let html = format!("<p style=\"font-size: {}pt\">Text</p>", size);
        let result = ironpress::html_to_pdf(&html);
        prop_assert!(result.is_ok());
    }

    /// Color values should be handled without panic
    #[test]
    fn color_values_never_panic(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255) {
        let html = format!("<p style=\"color: rgb({},{},{})\">Text</p>", r, g, b);
        let result = ironpress::html_to_pdf(&html);
        prop_assert!(result.is_ok());
    }
}
