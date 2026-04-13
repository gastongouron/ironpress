#!/usr/bin/env bash
# Generate Chromium reference PDFs and convert ALL pages to PNG for comparison.
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

        if [ ! -f "$ref_pdf" ]; then
            continue
        fi

        # Get page count
        page_count=$(pdftoppm -r 10 -png "$ref_pdf" /tmp/ref_count_ 2>/dev/null; ls /tmp/ref_count_*.png 2>/dev/null | wc -l; rm -f /tmp/ref_count_*.png)
        [ "$page_count" -lt 1 ] && page_count=1

        # Convert each page to PNG at 150 DPI
        for page in $(seq 1 "$page_count"); do
            if [ "$page" -eq 1 ]; then
                out_base="$name"
            else
                out_base="${name}-p${page}"
            fi
            ref_png="$REF_DIR/$layer/${out_base}.png"

            pdftoppm -r 150 -png -f "$page" -l "$page" "$ref_pdf" "$REF_DIR/$layer/${out_base}" 2>/dev/null
            # Rename from pdftoppm's suffix format to clean name
            for candidate in "$REF_DIR/$layer/${out_base}-${page}.png" "$REF_DIR/$layer/${out_base}-0${page}.png" "$REF_DIR/$layer/${out_base}-00${page}.png"; do
                if [ -f "$candidate" ]; then
                    mv "$candidate" "$ref_png"
                    break
                fi
            done
            [ -f "$ref_png" ] && count=$((count + 1))
        done

        rm -f "$ref_pdf"  # Clean up intermediate PDF
    done
done

echo "Done. $count reference PNGs saved to $REF_DIR"
