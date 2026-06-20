#!/usr/bin/env bash
#
# parity.sh — convenience runner for the feature-parity engine.
#
# Runs the in-process parity gate (renders every fixture, diffs against the
# committed Chrome references, rewrites report.json + REPORT.md, and enforces the
# regression gate). Extra args are passed through to the test binary.
#
# Usage:
#   scripts/parity.sh
#   scripts/parity.sh --test-threads=1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cargo test --manifest-path "$ROOT/Cargo.toml" --test feature_parity -- --nocapture "$@"

echo
echo "parity: scorecard written to $ROOT/tests/parity/REPORT.md (machine: report.json)"
