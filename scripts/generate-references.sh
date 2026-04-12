#!/usr/bin/env bash
# Generate Chromium reference PDFs and convert page 1 to PNG for comparison.
# Uses --print-to-pdf so Chrome applies the same A4 page constraints as ironpress.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
FIXTURES_DIR="$REPO_DIR/tests/fixtures"
REF_DIR="$FIXTURES_DIR/references"

# Find Chrome
CHROME=$(command -v google-chrome-stable || command -v google-chrome || command -v chromium || command -v chromium-browser || echo "")
if [ -z "$CHROME" ]; then
    echo "Warning: Chrome/Chromium not found — skipping reference generation"
    exit 0
fi

# Check for pdftoppm (needed to convert reference PDFs to PNGs)
if ! command -v pdftoppm &>/dev/null; then
    echo "Warning: pdftoppm not found — install poppler-utils"
    exit 0
fi

echo "Using: $CHROME"
count=0

for layer in features combined edge-cases; do
    mkdir -p "$REF_DIR/$layer"
    for html_file in "$FIXTURES_DIR/$layer"/*.html; do
        [ -f "$html_file" ] || continue
        name=$(basename "$html_file" .html)
        ref_pdf="$REF_DIR/$layer/$name.pdf"
        ref_png="$REF_DIR/$layer/$name.png"

        echo "  $layer/$name..."

        # Print to PDF with A4 page size (same as ironpress default)
        "$CHROME" --headless=new --disable-gpu --no-sandbox --disable-software-rasterizer \
            --print-to-pdf="$ref_pdf" \
            --no-pdf-header-footer \
            "file://$html_file" 2>/dev/null || \
        "$CHROME" --headless --disable-gpu --no-sandbox \
            --print-to-pdf="$ref_pdf" \
            --no-pdf-header-footer \
            "file://$html_file" 2>/dev/null || {
            echo "    WARN: failed to render $layer/$name"
            continue
        }

        # Convert page 1 of reference PDF to PNG at 150 DPI
        if [ -f "$ref_pdf" ]; then
            pdftoppm -r 150 -png -f 1 -l 1 "$ref_pdf" "$REF_DIR/$layer/$name" 2>/dev/null
            # Rename to consistent name (pdftoppm adds -1 or -01 suffix)
            for candidate in "$REF_DIR/$layer/${name}-1.png" "$REF_DIR/$layer/${name}-01.png" "$REF_DIR/$layer/${name}-001.png"; do
                if [ -f "$candidate" ]; then
                    mv "$candidate" "$ref_png"
                    break
                fi
            done
            rm -f "$ref_pdf"  # Clean up intermediate PDF
            [ -f "$ref_png" ] && count=$((count + 1))
        fi
    done
done

echo "Done. $count reference PNGs saved to $REF_DIR"
