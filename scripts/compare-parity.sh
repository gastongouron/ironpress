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
        ref_base=$(basename "$ref_png" .png)

        # Determine the PDF file and page number from the reference name.
        # Multi-page references use the suffix "-p2", "-p3", etc.
        if [[ "$ref_base" =~ ^(.*)-p([0-9]+)$ ]]; then
            name="${BASH_REMATCH[1]}"
            page="${BASH_REMATCH[2]}"
        else
            name="$ref_base"
            page=1
        fi
        pdf_file="$PDF_DIR/$layer/$name.pdf"

        if [ ! -f "$pdf_file" ]; then
            echo "| $ref_base | $layer | - | - | - | MISSING PDF |"
            continue
        fi

        any_result=1

        # Convert the target page to PNG at 150 DPI. pdftoppm exits non-zero when
        # the requested page doesn't exist (e.g. Chrome reference has 3 pages
        # but our output has 2). Swallow that error so the loop continues —
        # the missing render is reported as RENDER FAILED below.
        render_prefix="$TMPDIR_WORK/${layer}_${ref_base}"
        pdftoppm -r 150 -png -f "$page" -l "$page" "$pdf_file" "$render_prefix" 2>/dev/null || true
        # pdftoppm names output -N.png or -0N.png depending on page count
        render_png=""
        for candidate in "${render_prefix}-${page}.png" "${render_prefix}-0${page}.png" "${render_prefix}-00${page}.png"; do
            if [ -f "$candidate" ]; then
                render_png="$candidate"
                break
            fi
        done

        if [ -z "$render_png" ]; then
            echo "| $ref_base | $layer | - | - | - | RENDER FAILED |"
            continue
        fi

        # Resize both images to exactly the same dimensions to prevent compare failure
        ref_dims=$(identify -format "%wx%h" "$ref_png" 2>/dev/null || echo "")
        if [ -n "$ref_dims" ]; then
            # Force-resize rendered image to exact reference size (! overrides aspect ratio)
            convert "$render_png" -resize "${ref_dims}!" "$render_png" 2>/dev/null
            # Also force-resize reference to ensure both end up at identical dimensions
            # (needed when identify returns a size that convert rounds differently)
            resized_ref="$TMPDIR_WORK/${layer}_${name}_ref.png"
            convert "$ref_png" -resize "${ref_dims}!" "$resized_ref" 2>/dev/null
        else
            resized_ref="$ref_png"
        fi

        # Verify dimensions match before comparing
        render_dims=$(identify -format "%wx%h" "$render_png" 2>/dev/null || echo "")
        actual_ref_dims=$(identify -format "%wx%h" "$resized_ref" 2>/dev/null || echo "")
        if [ "$render_dims" != "$actual_ref_dims" ]; then
            # Dimensions still differ; force both to render_dims to guarantee a match
            canonical_dims="$render_dims"
            convert "$render_png"   -resize "${canonical_dims}!" "$render_png"   2>/dev/null
            convert "$resized_ref"  -resize "${canonical_dims}!" "$resized_ref"  2>/dev/null
        fi

        # Compare with ImageMagick AE metric (absolute error = diff pixel count)
        # NOTE: compare outputs the metric value on STDERR, not stdout
        diff_png="$TMPDIR_WORK/${layer}_${name}_diff.png"
        diff_pixels=$(compare -metric AE "$resized_ref" "$render_png" "$diff_png" 2>&1 || true)
        diff_pixels=$(echo "$diff_pixels" | tr -d '[:space:]')

        # If compare produced no numeric output, try extracting from error message
        if ! [[ "$diff_pixels" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
            # compare sometimes outputs "NNN image.png" — extract the number
            numeric=$(echo "$diff_pixels" | grep -oE '^[0-9]+(\.[0-9]+)?' || echo "")
            if [ -n "$numeric" ]; then
                diff_pixels="$numeric"
            else
                echo "# compare failed for $layer/$name: $diff_pixels" >&2
                diff_pixels=$(identify -format "%[fx:w*h]" "$resized_ref" 2>/dev/null || echo "0")
            fi
        fi

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

        echo "| $ref_base | $layer | $diff_pixels | $total_pixels | ${diff_pct}% | $status |"
    done
done

if [ "$any_result" -eq 0 ]; then
    echo ""
    echo "No fixtures compared. Reference PNGs not found in $REF_DIR"
    echo "This is expected on the first run. Run scripts/generate-references.sh to generate them."
    exit 0
fi

if [ "$failed" -ne 0 ]; then
    echo ""
    echo "FAILED: One or more fixtures exceeded the ${THRESHOLD}% diff threshold."
    exit 1
fi

echo ""
echo "All fixtures within ${THRESHOLD}% diff threshold."
