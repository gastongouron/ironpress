#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Wrap CSS in a minimal HTML document
        let html = format!("<style>{}</style><p>test</p>", s);
        let _ = ironpress::html_to_pdf(&html);
    }
});
