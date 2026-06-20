//! Feature-parity integration test entry point.
//!
//! Run with: `cargo test --test feature_parity -- --nocapture`
//!
//! The engine renders every fixture under `tests/parity/cases/**` in-process
//! through the `ironpress` library at Chrome-matching geometry, rasterizes the
//! result, diffs it against a committed Chrome reference PNG, scores parity, and
//! enforces a regression gate against the committed `tests/parity/report.json`.
//!
//! See `tests/parity/README.md` for the full workflow.

#[path = "parity_support/mod.rs"]
mod parity_support;

#[test]
fn feature_parity() {
    parity_support::run().expect("parity engine failure");
}
