#!/usr/bin/env bash
#
# parity-gen-coords.sh — Chrome-derived COORDINATE SIDECAR generator (Phase 2b).
#
# For each TARGET fixture under tests/parity/cases/<category>/<id>.html this
# renders a PDF with headless Chromium (same flags as parity-gen-refs.sh: US
# Letter, --no-pdf-header-footer, Chrome's default 0.4in margins), then extracts
# the COLORED vector geometry from Chrome's content stream — composing Chrome's
# nested `cm` stack (`.24 scale+Yflip` then `3.125 .. 115.625` translate; the net
# px->pt factor is 0.24*3.125 = 0.75) to absolute, top-left-origin PDF points —
# and writes a renderer-INDEPENDENT sidecar
#   tests/parity/coords/<category>/<id>.json
# matching tests/parity_support/verify/coords.rs (CoordSidecar).
#
# Sidecars are the COMMITTED "required result" the PdfGeometry verifier asserts
# any renderer's PDF against (spec §2.2/§2.4). They are CHROME-DERIVED GROUND
# TRUTH — never authored from ironpress's own (possibly wrong) output.
#
# v1 SCOPE (orchestrator brief):
#   * Only box/FILL geometry (and, where cleanly recoverable, the outer border
#     rect) is asserted. Text-run assertions are OUT of scope — raster owns text
#     Presence/Appearance — so text_runs is always [].
#   * Generate ONLY for the starter set: the probes + block-box-model + grid +
#     flexbox (solid colored boxes, Chrome PDF cleanly parseable). Gradient /
#     image / text-heavy / transform / filter categories are skipped (the
#     verifier degrades to raster there: applies()=false, no sidecar => no-op).
#   * Filter out the full-page white background rect(s) and near-white fills.
#   * A fixture whose Chrome PDF yields NO parseable colored boxes (image/text
#     only) gets NO sidecar (it stays raster-only).
#
# Snap-packaged Chromium CANNOT write --print-to-pdf to /tmp; all scratch lives
# under target/parity-coords-tmp (mirrors parity-gen-refs.sh's target/ usage).
#
# Usage:
#   scripts/parity-gen-coords.sh                 # all target categories
#   scripts/parity-gen-coords.sh grid            # only one target category
#   FORCE=1 scripts/parity-gen-coords.sh         # (no-op flag; sidecars always rewritten)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARITY="$ROOT/tests/parity"
CASES="$PARITY/cases"
COORDS="$PARITY/coords"
FONTS="$PARITY/fonts"
TMP="$ROOT/target/parity-coords-tmp"
mkdir -p "$TMP" "$COORDS"

# The starter set: ONLY these categories get sidecars in v1 (clean box geometry).
TARGET_CATEGORIES=(probes block-box-model grid flexbox)

ONLY_CATEGORY=""
for arg in "$@"; do
  case "$arg" in
    -*) echo "unknown flag: $arg" >&2; exit 2 ;;
    *) ONLY_CATEGORY="$arg" ;;
  esac
done

# --- deterministic fonts (mirror parity-gen-refs.sh) -------------------------
# Only matters for text fixtures (out of scope here), but keep Chrome consistent.
if [ -f "$FONTS/fonts.conf" ]; then
  export FONTCONFIG_FILE="$FONTS/fonts.conf"
  export FONTCONFIG_PATH="$FONTS"
fi

# --- locate chromium ---------------------------------------------------------
CHROMIUM=""
for cand in chromium-browser /snap/bin/chromium chromium google-chrome google-chrome-stable; do
  if command -v "$cand" >/dev/null 2>&1; then CHROMIUM="$cand"; break; fi
done
if [ -z "$CHROMIUM" ]; then
  echo "parity-gen-coords: chromium not found; cannot generate sidecars." >&2
  exit 1
fi
if [ ! -d "$CASES" ]; then
  echo "parity-gen-coords: no cases dir at $CASES (nothing to do)." >&2
  exit 0
fi

echo "parity-gen-coords: chromium='$CHROMIUM', only='${ONLY_CATEGORY:-<starter set>}'"

# render_chrome_pdf <abs_html> <out_pdf>  — Chrome PDF with retries (mirrors the
# robustness of parity-gen-refs.sh: private profile per attempt, timeout, reap).
render_chrome_pdf() {
  local html="$1" out="$2" udd attempt
  for attempt in 1 2 3; do
    udd="$(mktemp -d "$TMP/udd.XXXXXX")"
    timeout -k 5s 60s "$CHROMIUM" --headless=new --disable-gpu --no-sandbox \
      --disable-software-rasterizer --user-data-dir="$udd" \
      --no-pdf-header-footer --print-to-pdf="$out" "file://$html" >/dev/null 2>&1 || true
    pkill -9 -f "$udd" 2>/dev/null || true
    rm -rf "$udd"
    [ -s "$out" ] && return 0
    sleep 0.4
  done
  return 1
}

gen_one() {
  local html="$1" rel category base out json
  rel="${html#"$CASES"/}"          # <category>/<id>.html
  category="${rel%%/*}"
  base="$(basename "$html" .html)" # <id>
  out="$TMP/$base.coords.pdf"
  json="$COORDS/$category/$base.json"
  mkdir -p "$COORDS/$category"

  if ! render_chrome_pdf "$html" "$out"; then
    echo "  FAILED render: $category/$base" >&2
    return 1
  fi

  # Extract + emit sidecar. The python writes the JSON (or prints SKIP on stdout
  # when Chrome yields no parseable colored box, in which case NO file is written).
  local result
  result="$(python3 "$SCRIPT_DIR/parity_coords_extract.py" "$out" "$json" "$category/$base")"
  echo "  $result"
  rm -f "$out"
}

GENERATED=0 SKIPPED=0 FAILED=0
process_category() {
  local cat="$1"
  [ -d "$CASES/$cat" ] || { echo "parity-gen-coords: no cases for category '$cat'" >&2; return 0; }
  local html
  while IFS= read -r -d '' html; do
    if gen_one "$html"; then :; fi
  done < <(find "$CASES/$cat" -type f -name '*.html' -print0 | sort -z)
}

if [ -n "$ONLY_CATEGORY" ]; then
  process_category "$ONLY_CATEGORY"
else
  for cat in "${TARGET_CATEGORIES[@]}"; do
    process_category "$cat"
  done
fi

# Tally by re-reading what the python emitted (it prints WROTE/SKIP/EMPTY lines).
echo "parity-gen-coords: done. Run wrote sidecars under $COORDS/"

# --- coords.lock -------------------------------------------------------------
# Record exactly which fixture CONTENT each committed sidecar corresponds to:
# a pretty JSON map {id: sha256(cases/<category>/<id>.html)} mirroring refs.lock,
# but ONLY for fixtures that actually have a sidecar on disk. gate.rs
# check_coords_freshness recomputes these and surfaces a sidecar that has drifted
# from its fixture (non-gating locally, CI-enforced).
write_coords_lock() {
  local lock="$PARITY/coords.lock"
  local manifest_dir="$PARITY/manifest"
  [ -d "$manifest_dir" ] || { echo "parity-gen-coords: no manifest dir; skipping coords.lock." >&2; return 0; }

  PARITY_DIR="$PARITY" COORDS_DIR="$COORDS" python3 - "$lock" <<'PY'
import glob, hashlib, json, os, sys

lock_path = sys.argv[1]
parity = os.environ["PARITY_DIR"]
coords = os.environ["COORDS_DIR"]
manifest_glob = os.path.join(parity, "manifest", "*.json")

mapping = {}
for mf in sorted(glob.glob(manifest_glob)):
    with open(mf, "r", encoding="utf-8") as fh:
        entries = json.load(fh)
    for e in entries:
        fid = e["id"]
        category = e["category"]
        rel = e["file"]  # cases/<category>/<id>.html
        # Only lock fixtures that actually have a committed sidecar.
        sidecar = os.path.join(coords, category, f"{fid}.json")
        if not os.path.exists(sidecar):
            continue
        html_path = os.path.join(parity, rel)
        try:
            with open(html_path, "rb") as hf:
                mapping[fid] = hashlib.sha256(hf.read()).hexdigest()
        except FileNotFoundError:
            print(f"parity-gen-coords: WARNING fixture missing for id={fid}: {rel}", file=sys.stderr)

ordered = {k: mapping[k] for k in sorted(mapping)}
with open(lock_path, "w", encoding="utf-8") as out:
    json.dump(ordered, out, indent=2, sort_keys=True)
    out.write("\n")
print(f"parity-gen-coords: wrote coords.lock ({len(ordered)} entries)")
PY
}
write_coords_lock
