//! Small dependency-free helpers shared across the engine: rounding, byte-slice
//! search, comment stripping, SHA-256 hex, and the `pdftoppm` presence probe.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::process::Command;

pub(crate) fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
pub(crate) fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

pub(crate) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|w| w == needle)
}

/// Remove CSS (`/* ... */`) and HTML (`<!-- ... -->`) comment spans so that
/// keyword guards (e.g. the `@page` check) only see live markup, not prose.
pub(crate) fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // CSS comment: skip to closing */
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if bytes[i] == b'<' && bytes[i..].starts_with(b"<!--") {
            // HTML comment: skip to closing -->
            i += 4;
            while i < bytes.len() && !bytes[i..].starts_with(b"-->") {
                i += 1;
            }
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lowercase-hex SHA-256 of arbitrary bytes (fixture HTML hashing for refs.lock).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

pub(crate) fn which(bin: &str) -> bool {
    Command::new(bin)
        .arg("-v")
        .output()
        .map(|_| true)
        .unwrap_or(false)
        || Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}
