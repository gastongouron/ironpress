#!/usr/bin/env bash
# Usage: ./scripts/compare-parity.sh <pdf-dir> [threshold-percent]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
REF_DIR="$REPO_DIR/tests/fixtures/references"

PDF_DIR="${1:-}"
THRESHOLD="${2:-5}"

if [ -z "$PDF_DIR" ]; then
    echo "Usage: $0 <pdf-dir> [threshold-percent]"
    echo "  pdf-dir            Directory containing rendered PDF files"
    echo "  threshold-percent  Max allowed diff percentage (default: 5)"
    exit 1
fi

if [ ! -d "$PDF_DIR" ]; then
    echo "Error: PDF directory not found: $PDF_DIR"
    exit 1
fi

# Check dependencies
for cmd in pdftoppm compare convert; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "Error: '$cmd' not found. Install poppler-utils and ImageMagick."
        exit 1
    fi
done

TMPDIR_WORK=$(mktemp -d)
trap 'rm -rf "$TMPDIR_WORK"' EXIT

failed=0
any_result=0

echo "| Fixture | Layer | Diff Pixels | Total Pixels | Diff % | Status |"
echo "|---------|-------|-------------|--------------|--------|--------|"

for layer in features combined edge-cases; do
    ref_layer_dir="$REF_DIR/$layer"
    if [ ! -d "$ref_layer_dir" ]; then
        continue
    fi

    for ref_png in "$ref_layer_dir"/*.png; do
        [ -f "$ref_png" ] || continue
        name=$(basename "$ref_png" .png)
        pdf_file="$PDF_DIR/$name.pdf"

        if [ ! -f "$pdf_file" ]; then
            echo "| $name | $layer | - | - | - | MISSING PDF |"
            continue
        fi

        any_result=1

        # Convert PDF page 1 to PNG at 150 DPI
        render_prefix="$TMPDIR_WORK/${layer}_${name}"
        pdftoppm -r 150 -png -f 1 -l 1 "$pdf_file" "$render_prefix" 2>/dev/null
        render_png="${render_prefix}-1.png"

        if [ ! -f "$render_png" ]; then
            echo "| $name | $layer | - | - | - | RENDER FAILED |"
            continue
        fi

        # Resize rendered PNG to match reference dimensions if needed
        ref_dims=$(identify -format "%wx%h" "$ref_png" 2>/dev/null || echo "")
        if [ -n "$ref_dims" ]; then
            convert "$render_png" -resize "$ref_dims!" "$render_png" 2>/dev/null
        fi

        # Compare with ImageMagick AE metric (absolute error = diff pixel count)
        diff_png="$TMPDIR_WORK/${layer}_${name}_diff.png"
        diff_pixels=$(compare -metric AE "$ref_png" "$render_png" "$diff_png" 2>&1 || true)
        # compare exits non-zero when images differ; capture output regardless
        diff_pixels=$(echo "$diff_pixels" | tr -d '[:space:]')

        # Calculate total pixels from reference
        total_pixels=$(identify -format "%[fx:w*h]" "$ref_png" 2>/dev/null || echo "1")

        # Compute percentage (use awk for floating point)
        diff_pct=$(awk "BEGIN { printf \"%.2f\", ($diff_pixels / $total_pixels) * 100 }" 2>/dev/null || echo "N/A")

        # Determine pass/fail
        status="PASS"
        if awk "BEGIN { exit ($diff_pct > $THRESHOLD) ? 0 : 1 }" 2>/dev/null; then
            status="FAIL"
            failed=1
        fi

        echo "| $name | $layer | $diff_pixels | $total_pixels | ${diff_pct}% | $status |"
    done
done

if [ "$any_result" -eq 0 ]; then
    echo ""
    echo "No fixtures compared. Check that reference PNGs exist in $REF_DIR"
    echo "Run scripts/generate-references.sh first."
    exit 1
fi

if [ "$failed" -ne 0 ]; then
    echo ""
    echo "FAILED: One or more fixtures exceeded the ${THRESHOLD}% diff threshold."
    exit 1
fi

echo ""
echo "All fixtures within ${THRESHOLD}% diff threshold."
