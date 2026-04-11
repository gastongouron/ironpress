#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
FIXTURES_DIR="$REPO_DIR/tests/fixtures"
REF_DIR="$FIXTURES_DIR/references"

# Find Chrome
CHROME=$(command -v google-chrome || command -v chromium || command -v chromium-browser || echo "")
if [ -z "$CHROME" ]; then
    echo "Error: Chrome/Chromium not found"
    exit 1
fi

for layer in features combined edge-cases; do
    mkdir -p "$REF_DIR/$layer"
    for html_file in "$FIXTURES_DIR/$layer"/*.html; do
        name=$(basename "$html_file" .html)
        output="$REF_DIR/$layer/$name.png"
        echo "Rendering $layer/$name..."
        "$CHROME" --headless --disable-gpu --no-sandbox \
            --window-size=1240,1754 \
            --screenshot="$output" \
            "file://$html_file" 2>/dev/null
    done
done
echo "Done. References saved to $REF_DIR"
