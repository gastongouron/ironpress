#!/usr/bin/env bash
#
# parity-gen-refs.sh — one-time Chrome reference generator for the parity engine.
#
# For each fixture under tests/parity/cases/<category>/<id>.html this renders a
# PDF with headless Chromium at US Letter + 0.4in (28.8pt) margins (Chrome's
# --print-to-pdf defaults, matching the ironpress lib render in the engine), then
# rasterizes page 1 with pdftoppm at the engine DPI to
# tests/parity/refs/<category>/<id>.png.
#
# Idempotent: existing refs are skipped unless --force / FORCE=1. An optional
# positional <category> argument limits generation to one bucket. Chrome is never
# run at test time; this script is the only place it is invoked.
#
# PARALLELISM: fixtures are rendered concurrently across a bounded pool of
# N = min(nproc-2, 8) workers (xargs -P N). Chromium cold-start dominates the
# wall-clock cost, so overlapping launches is the single biggest win. Each
# concurrent Chromium gets its OWN --user-data-dir (mktemp -d per job): Chromium
# refuses (or silently serializes behind a profile lock) when multiple instances
# share a profile, which would defeat the parallelism. Per-job temp dirs and the
# intermediate PDF are removed when the job finishes. The produced PNG set is
# byte-identical to the old serial version — only the order of console lines and
# the wall-clock time change.
#
# Usage:
#   scripts/parity-gen-refs.sh                # generate all missing refs
#   scripts/parity-gen-refs.sh --force        # regenerate everything
#   scripts/parity-gen-refs.sh flexbox        # only the flexbox bucket
#   FORCE=1 scripts/parity-gen-refs.sh grid   # force one bucket

set -euo pipefail

DPI=300

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PARITY="$ROOT/tests/parity"
CASES="$PARITY/cases"
REFS="$PARITY/refs"
FONTS="$PARITY/fonts"
TMP="$ROOT/target/parity-tmp"
mkdir -p "$TMP"

# --- Paged.js (spec-compliant paged-media reference layout) ------------------
# Chrome's native --print-to-pdf does NOT implement CSS Paged Media: its
# printable margin rounds to ~116px (vs the spec 120px@300dpi = 28.8pt, so
# content sits a VARIABLE 1-5 device px off the ironpress render) and @page
# margin boxes / named pages / fragmentation are unsupported. ironpress IS a spec
# paged renderer, so the reference must be laid out by a spec engine. We render
# the reference with pagedjs-cli (Paged.js + Puppeteer): it paginates per CSS
# Paged Media and, crucially, WAITS for the layout to finish before printing
# (the raw `chromium --print-to-pdf` + `--virtual-time-budget` path races the
# async pagination under parallel cold starts and emits blank pages). The
# page geometry is forced to ironpress's (Letter, 28.8pt margin). Set PAGEDJS=0
# to fall back to Chrome's native print (the legacy ad-hoc model).
PAGEDJS="${PAGEDJS:-0}"
PAGE_CSS="$TMP/pagedjs-page.css"
PAGEDJS_BIN=""
if [ "$PAGEDJS" = "1" ]; then
  printf '@page{size:Letter;margin:28.8pt;}\n' > "$PAGE_CSS"
  # Resolve (or bootstrap) pagedjs-cli. Prefer an already-installed binary; else
  # install it into target/pagedtool WITHOUT downloading a bundled Chromium
  # (PUPPETEER_SKIP_DOWNLOAD=1) — we drive the system Chromium instead.
  if command -v pagedjs-cli >/dev/null 2>&1; then
    PAGEDJS_BIN="$(command -v pagedjs-cli)"
  elif [ -x "$ROOT/target/pagedtool/node_modules/.bin/pagedjs-cli" ]; then
    PAGEDJS_BIN="$ROOT/target/pagedtool/node_modules/.bin/pagedjs-cli"
  elif command -v npm >/dev/null 2>&1; then
    echo "parity-gen-refs: installing pagedjs-cli (system Chromium, no bundled download)..." >&2
    if PUPPETEER_SKIP_DOWNLOAD=1 npm install --no-save --prefix "$ROOT/target/pagedtool" pagedjs-cli >/dev/null 2>&1 \
       && [ -x "$ROOT/target/pagedtool/node_modules/.bin/pagedjs-cli" ]; then
      PAGEDJS_BIN="$ROOT/target/pagedtool/node_modules/.bin/pagedjs-cli"
    fi
  fi
  if [ -z "$PAGEDJS_BIN" ]; then
    echo "parity-gen-refs: WARNING pagedjs-cli unavailable (need node+npm); falling back to native print." >&2
    PAGEDJS=0
  fi
fi

# --- deterministic fonts -----------------------------------------------------
# Point fontconfig (and therefore Chrome) at the bundled Parity faces so the
# reference rasters use the SAME outlines as the ironpress in-process render
# (which registers tests/parity/fonts/Parity*.ttf via HtmlConverter::add_font).
# Without this, Chrome would shape with whatever system fonts happen to be
# installed and text-bearing fixtures would measure noise, not parity.
if [ -f "$FONTS/fonts.conf" ]; then
  export FONTCONFIG_FILE="$FONTS/fonts.conf"
  export FONTCONFIG_PATH="$FONTS"
  mkdir -p /tmp/ironpress-parity-fontcache
  echo "parity-gen-refs: FONTCONFIG_FILE=$FONTCONFIG_FILE"
else
  echo "parity-gen-refs: WARNING $FONTS/fonts.conf missing; refs will use system fonts (text parity = noise)." >&2
fi

FORCE="${FORCE:-0}"
ONLY_CATEGORY=""
for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    -*) echo "unknown flag: $arg" >&2; exit 2 ;;
    *) ONLY_CATEGORY="$arg" ;;
  esac
done

# --- locate chromium ---------------------------------------------------------
CHROMIUM=""
for cand in chromium-browser /snap/bin/chromium chromium google-chrome google-chrome-stable; do
  if command -v "$cand" >/dev/null 2>&1; then CHROMIUM="$cand"; break; fi
done
if [ -z "$CHROMIUM" ]; then
  echo "parity-gen-refs: chromium not found; skipping reference generation." >&2
  echo "  (fixtures without a reference stay UNKNOWN and never fail CI.)" >&2
  exit 0
fi
# Absolute path for Puppeteer (pagedjs-cli): puppeteer's executablePath must be a
# real file, not a PATH name like "chromium-browser" (it errors "no executable
# found"). The native --print-to-pdf path is happy with either.
CHROMIUM_ABS="$(command -v "$CHROMIUM" 2>/dev/null || echo "$CHROMIUM")"

if ! command -v pdftoppm >/dev/null 2>&1; then
  echo "parity-gen-refs: pdftoppm (poppler) not found; skipping." >&2
  exit 0
fi

if [ ! -d "$CASES" ]; then
  echo "parity-gen-refs: no cases dir at $CASES (nothing to do)." >&2
  exit 0
fi

# --- size the worker pool ----------------------------------------------------
# N = min(nproc-2, 8). Leave two cores for the OS / Chromium helper threads and
# cap at 8 so we don't thrash memory with too many concurrent browsers.
NCPU="$(nproc 2>/dev/null || echo 4)"
JOBS=$((NCPU - 2))
[ "$JOBS" -lt 1 ] && JOBS=1
[ "$JOBS" -gt 8 ] && JOBS=8
# pagedjs-cli launches a full Puppeteer-driven Chromium PER job (~250MB + cold
# start), so cap concurrency lower than the lightweight --print-to-pdf path to
# avoid memory pressure / launch races that yield blank pages.
if [ "$PAGEDJS" = "1" ] && [ "$JOBS" -gt 4 ]; then JOBS=4; fi

echo "parity-gen-refs: chromium='$CHROMIUM', dpi=$DPI, force=$FORCE, only='${ONLY_CATEGORY:-<all>}', jobs=$JOBS"

# Make the bundled Parity faces discoverable by Chromium. CRITICAL: snap-packaged
# Chromium IGNORES $FONTCONFIG_FILE, so pointing it at tests/parity/fonts/fonts.conf
# is not enough — Chrome falls back to a serif and the text refs become wrong.
# Installing the faces into the user font dir (which the snap desktop interface
# exposes) + refreshing the cache makes Chrome AND ironpress select the same
# outlines. Idempotent; in CI this targets the ephemeral runner's font dir.
FONTS_SRC="$ROOT/tests/parity/fonts"
USER_FONTS="${XDG_DATA_HOME:-$HOME/.local/share}/fonts"
if ls "$FONTS_SRC"/Parity*.ttf >/dev/null 2>&1; then
  mkdir -p "$USER_FONTS"
  cp -f "$FONTS_SRC"/Parity*.ttf "$USER_FONTS"/ 2>/dev/null || true
  command -v fc-cache >/dev/null 2>&1 && fc-cache -f "$USER_FONTS" >/dev/null 2>&1 || true
  echo "parity-gen-refs: installed bundled Parity faces into $USER_FONTS"
fi

# --- per-job worker ----------------------------------------------------------
# render_one <html>  — runs in its own subshell (one xargs slot). Emits exactly
# one status token to stdout: "G" generated, "S" skipped, "F" failed. Human log
# lines go to stderr so they don't pollute the status stream.
render_one() {
  local html="$1"
  local rel category base ref pdf udd pages

  rel="${html#"$CASES"/}"           # <category>/<id>.html
  category="${rel%%/*}"
  base="$(basename "$html" .html)"  # <id>

  ref="$REFS/$category/$base.png"
  if [ -f "$ref" ] && [ "$FORCE" != "1" ]; then
    echo "S"
    return 0
  fi

  mkdir -p "$REFS/$category"

  # Per-fixture reference ORACLE (manifest `oracle`, default "chrome"). CSS GCPM
  # features (footnotes, running elements) use WeasyPrint because Chrome's print
  # path renders them blank — Chrome+Paged.js are not a valid oracle there.
  # "none" = no oracle exists; skip (the fixture stays UNKNOWN, the report shows
  # only the ironpress render).
  local oracle
  oracle="$(python3 -c "import json
try:
    d=json.load(open('$ROOT/tests/parity/manifest/$category.json'))
    items=d if isinstance(d,list) else d.get('fixtures',[])
    print(next((e.get('oracle','chrome') for e in items if e.get('id')=='$base'),'chrome'))
except Exception:
    print('chrome')" 2>/dev/null || echo chrome)"
  if [ "$oracle" = "none" ]; then
    echo "  no pixel oracle (skipped, UNKNOWN): $category/$base" >&2
    echo "S"
    return 0
  fi

  # Unique scratch per job: a private Chromium profile dir (mandatory for
  # concurrency — a shared profile lock serializes/aborts parallel runs) and a
  # unique intermediate PDF path. Both are cleaned up on every exit path.
  udd="$(mktemp -d "$TMP/udd.XXXXXX")"
  pdf="$(mktemp "$TMP/ref.XXXXXX.pdf")"
  local inj=""
  trap 'rm -rf "$udd" "$pdf" "$inj"' RETURN

  local ok=""
  if [ "$oracle" = "weasyprint" ]; then
    # CSS GCPM oracle. WeasyPrint honours float:footnote + position:running()/
    # element(), which Chrome's print path drops. It reads the fixture's own @page
    # geometry, the same box ironpress lays out against, so the rasters align.
    local attempt
    for attempt in 1 2 3; do
      timeout -k 5s 120s python3 -m weasyprint "$html" "$pdf" >/dev/null 2>&1 || true
      if [ -s "$pdf" ]; then ok=1; break; fi
      sleep 0.5
    done
  elif [ "$PAGEDJS" = "1" ]; then
    # Spec-compliant paged-media render via pagedjs-cli (Puppeteer driving the
    # system Chromium). pagedjs-cli WAITS for Paged.js's `rendered` event before
    # printing, so — unlike `chromium --print-to-pdf` + `--virtual-time-budget`,
    # which races Paged.js's async pagination under parallel cold starts and
    # silently emits BLANK pages — every page is fully laid out. `--style`
    # forces the ironpress page geometry (Letter, 28.8pt margin) so the @page box
    # matches `PageSize::LETTER` + `Margin::uniform(28.8)`. Retried a few times
    # (Chromium cold starts are occasionally flaky).
    local attempt
    for attempt in 1 2 3; do
      PUPPETEER_EXECUTABLE_PATH="$CHROMIUM_ABS" PUPPETEER_SKIP_DOWNLOAD=1 \
        timeout -k 5s 120s "$PAGEDJS_BIN" -i "$html" -o "$pdf" \
          --page-size Letter --style "$PAGE_CSS" \
          --browserArgs "--no-sandbox,--disable-gpu,--disable-software-rasterizer" \
          >/dev/null 2>&1 || true
      if [ -s "$pdf" ]; then ok=1; break; fi
      sleep 0.5
    done
  else
    # Native Chrome --print-to-pdf (the legacy ad-hoc print model; PAGEDJS=0).
    # Under concurrent snap-Chromium cold starts, --headless=new sporadically
    # aborts on a GPU-process namespace race; retry with a FRESH profile and a
    # short backoff, falling back to legacy --headless as a last resort.
    local attempt
    for attempt in 1 2 3; do
      rm -rf "$udd"; udd="$(mktemp -d "$TMP/udd.XXXXXX")"
      timeout -k 5s 60s "$CHROMIUM" --headless=new --disable-gpu --no-sandbox \
           --disable-software-rasterizer --user-data-dir="$udd" \
           --no-pdf-header-footer --print-to-pdf="$pdf" "file://$html" >/dev/null 2>&1
      pkill -9 -f "$udd" 2>/dev/null || true
      if [ -s "$pdf" ]; then ok=1; break; fi
      sleep 0.4
    done
    if [ -z "$ok" ]; then
      rm -rf "$udd"; udd="$(mktemp -d "$TMP/udd.XXXXXX")"
      timeout -k 5s 60s "$CHROMIUM" --headless --disable-gpu --no-sandbox \
        --disable-software-rasterizer --user-data-dir="$udd" \
        --no-pdf-header-footer --print-to-pdf="$pdf" "file://$html" >/dev/null 2>&1 || true
      pkill -9 -f "$udd" 2>/dev/null || true
    fi
  fi

  if [ ! -s "$pdf" ]; then
    echo "  FAILED render after retries: $category/$base" >&2
    echo "F"
    return 0
  fi

  # Rasterize ALL pages so pagination is actually testable: page 1 -> <id>.png
  # (the legacy single-page name, so the whole existing corpus is untouched),
  # pages 2.. -> <id>.pN.png. The engine's multi-page comparison asserts the page
  # COUNT matches Chrome and diffs every page, so references are no longer forced
  # to a single page (the old "shrink to one page" constraint hid real pagination
  # diffs — e.g. a trailing blank page). Render to a TEMP prefix, then atomically
  # move each page into place so an interrupted run never leaves a partial ref.
  local refdir="$REFS/$category"
  # Drop stale extra-page refs for this id first (a fixture's page count may shrink).
  rm -f "$refdir/$base".p[0-9]*.png 2>/dev/null || true
  local tmp_prefix; tmp_prefix="$(mktemp -u "$TMP/png.XXXXXX")"
  if timeout 90s pdftoppm -r "$DPI" -png "$pdf" "$tmp_prefix" 2>/dev/null; then
    # pdftoppm (no -singlefile) writes <prefix>-N.png, zero-padded to the page
    # count's width; collect in natural numeric order.
    local pages_list; pages_list="$(ls "$tmp_prefix"-*.png 2>/dev/null | sort -V)"
    if [ -z "$pages_list" ]; then
      echo "  FAILED rasterize (no pages): $category/$base" >&2
      echo "F"
      return 0
    fi
    # Blank-page guard on page 1: a uniform (all-white) raster means no content
    # (e.g. a render race). A completely uniform image has standard-deviation 0.
    local p1; p1="$(printf '%s\n' "$pages_list" | head -1)"
    local sd="1"
    if command -v identify >/dev/null 2>&1; then
      sd="$(identify -format '%[standard-deviation]' "$p1" 2>/dev/null || echo 1)"
    fi
    if [ "${sd%%.*}" = "0" ] && [ "$sd" != "1" ]; then
      # shellcheck disable=SC2086
      rm -f $pages_list 2>/dev/null || true
      echo "  BLANK render (rejected, kept existing ref): $category/$base" >&2
      echo "F"
      return 0
    fi
    # Move page 1 -> <id>.png, pages 2.. -> <id>.pN.png.
    local n=0 f
    while IFS= read -r f; do
      n=$((n + 1))
      if [ "$n" -eq 1 ]; then
        mv -f "$f" "$ref"
      else
        mv -f "$f" "$refdir/$base.p$n.png"
      fi
    done <<< "$pages_list"
    echo "  generated $category/$base.png ($n page(s))" >&2
    echo "G"
  else
    # shellcheck disable=SC2086
    rm -f "$tmp_prefix"-*.png 2>/dev/null || true
    echo "  FAILED rasterize: $category/$base" >&2
    echo "F"
  fi
}

# Export everything the worker subshells need.
export -f render_one
export CHROMIUM CHROMIUM_ABS CASES REFS TMP FORCE DPI ROOT PAGEDJS PAGEDJS_BIN PAGE_CSS

# --- dispatch concurrently ---------------------------------------------------
# Collect the matching fixtures (NUL-delimited, sorted for stable selection),
# then fan them out over the bounded pool. Each xargs slot is a fresh bash that
# runs render_one on a single fixture, so the per-job mktemp profiles never
# collide. We tally the one-letter status tokens streamed back on stdout.
status_file="$(mktemp "$TMP/status.XXXXXX")"
trap 'rm -f "$status_file"' EXIT

select_fixtures() {
  if [ -n "$ONLY_CATEGORY" ]; then
    find "$CASES/$ONLY_CATEGORY" -type f -name '*.html' -print0 2>/dev/null | sort -z
  else
    find "$CASES" -type f -name '*.html' -print0 | sort -z
  fi
}

select_fixtures | xargs -0 -P "$JOBS" -I {} bash -c 'render_one "$@"' _ {} > "$status_file"

generated="$(grep -c '^G$' "$status_file" || true)"
skipped="$(grep -c '^S$' "$status_file" || true)"
failed="$(grep -c '^F$' "$status_file" || true)"

echo "parity-gen-refs: done — generated=$generated skipped=$skipped failed=$failed"

# --- refs.lock --------------------------------------------------------------
# Record exactly which fixture CONTENT the committed references correspond to:
# a pretty-printed JSON object mapping each fixture id -> sha256 of its
# cases/<category>/<id>.html. CI's refs-freshness gate (parity.yml) recomputes
# these hashes and FAILS if a fixture changed without its reference (and this
# lock) being regenerated — that is what forces `scripts/parity-gen-refs.sh
# --force` whenever fixtures change.
#
# We regenerate the WHOLE lock from every MANIFEST entry on each run (simpler and
# always self-consistent) rather than patching only the ids touched this run. The
# key is the manifest `id` and the value is sha256 of the fixture it points at
# (`file` = cases/<category>/<id>.html). NOTE: the manifest id is NOT always the
# html basename — e.g. cases/backgrounds-borders/box-shadow-offset.html has id
# `border-box-shadow-offset` — so we MUST read the mapping from the manifests, not
# infer it from filenames, otherwise the freshness gate would key on the wrong id.
write_refs_lock() {
  local lock="$PARITY/refs.lock"
  local manifest_dir="$PARITY/manifest"

  if [ ! -d "$manifest_dir" ]; then
    echo "parity-gen-refs: no manifest dir at $manifest_dir; skipping refs.lock." >&2
    return 0
  fi

  # Python builds the {id: sha256(file)} map from all manifests and writes pretty,
  # key-sorted JSON. sha256 is computed over the exact bytes of each fixture html.
  PARITY_DIR="$PARITY" python3 - "$lock" <<'PY'
import glob, hashlib, json, os, sys

lock_path = sys.argv[1]
parity = os.environ["PARITY_DIR"]
manifest_glob = os.path.join(parity, "manifest", "*.json")

mapping = {}
for mf in sorted(glob.glob(manifest_glob)):
    with open(mf, "r", encoding="utf-8") as fh:
        entries = json.load(fh)
    for e in entries:
        fid = e["id"]
        rel = e["file"]  # cases/<category>/<id>.html
        html_path = os.path.join(parity, rel)
        try:
            with open(html_path, "rb") as hf:
                digest = hashlib.sha256(hf.read()).hexdigest()
        except FileNotFoundError:
            print(f"parity-gen-refs: WARNING fixture missing for id={fid}: {rel}",
                  file=sys.stderr)
            continue
        mapping[fid] = digest

ordered = {k: mapping[k] for k in sorted(mapping)}
with open(lock_path, "w", encoding="utf-8") as out:
    json.dump(ordered, out, indent=2, sort_keys=True)
    out.write("\n")

print(f"wrote refs.lock ({len(ordered)} entries)")
PY
}

write_refs_lock
