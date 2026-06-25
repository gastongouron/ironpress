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
    haystack.windows(needle.len()).any(|w| w == needle)
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
