#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Fuzz CSS + table/flex structures to exercise v0.9 features:
        // flex-grow/shrink/basis, margin collapsing, descendant selectors
        let html = format!(
            r#"<html><head><style>
            .row {{ display: flex; }}
            .total td {{ font-weight: bold; }}
            {css}
            </style></head><body>
            <div class="row"><div>A</div><div>B</div></div>
            <table><tbody>
            <tr><td>X</td></tr>
            <tr class="total"><td>Y</td></tr>
            </tbody></table>
            </body></html>"#,
            css = s
        );
        let _ = ironpress::html_to_pdf(&html);
    }
});
