#!/usr/bin/env bash
# Render all HTML fixtures to PDF using ironpress CLI.
# Usage: ./scripts/render-fixtures.sh <output-dir>
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
FIXTURES_DIR="$REPO_DIR/tests/fixtures"
OUTPUT_DIR="${1:-/tmp/parity-pdfs}"

# Build ironpress in release mode
echo "Building ironpress (release)..."
cargo build --release --manifest-path="$REPO_DIR/Cargo.toml" 2>/dev/null
CLI="$REPO_DIR/target/release/ironpress"

if [ ! -f "$CLI" ]; then
    echo "Error: ironpress binary not found at $CLI"
    exit 1
fi

for layer in features combined edge-cases; do
    mkdir -p "$OUTPUT_DIR/$layer"
    for html_file in "$FIXTURES_DIR/$layer"/*.html; do
        [ -f "$html_file" ] || continue
        name=$(basename "$html_file" .html)
        output="$OUTPUT_DIR/$layer/$name.pdf"
        echo "  $layer/$name..."
        "$CLI" --margin 56 "$html_file" "$output" 2>/dev/null || echo "    WARN: failed to render $layer/$name"
    done
done

echo "Done. PDFs saved to $OUTPUT_DIR"
echo "Total: $(find "$OUTPUT_DIR" -name "*.pdf" | wc -l | tr -d ' ') files"
