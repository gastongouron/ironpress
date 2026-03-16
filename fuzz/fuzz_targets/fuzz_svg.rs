#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Embed fuzzed SVG content inside an HTML document
        let html = format!(
            "<html><body><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">{}</svg></body></html>",
            s
        );
        let _ = ironpress::html_to_pdf(&html);
    }
});
