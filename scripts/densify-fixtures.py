#!/usr/bin/env python3
"""Well-defined + page-sized fixture transform (parity harness).

For each fixture this:
  1. Injects shared GLOBAL DEFAULTS (`html { font-family: ParitySans;
     line-height: 1.5 }`) so every fixture is calibrated to one well-defined
     baseline. Both are otherwise implementation-defined (generic family mapping;
     Blink rounds hhea metrics for line-height:normal, ironpress uses usWin), so
     pinning them makes ironpress and Chrome render identically. Element-level
     rules still override the defaults (feature tests for a specific font /
     line-height keep working).
  2. Normalizes any explicit non-bundled / UA-discretionary family token in a
     font-family value (sans-serif / serif / monospace / 'Parity Sans' /
     ParityCustom) to a deterministic bundled bare Parity face. Bare ParitySans
     resolves to the bundled face in BOTH engines (ironpress add_font; Chrome via
     the gen-refs font install) — verified with pdffonts — so no @font-face is
     needed.
  3. Sizes @page to the rendered content extent with margin:0 so the page is
     (almost) all content, not white space, and the page-origin offset is 0
     (content sits at the page origin in both engines).

Usage:
  densify-fixtures.py <fixture.html> [...]      # transform specific fixtures
  densify-fixtures.py --all                      # transform every eligible fixture
  densify-fixtures.py --category <cat>           # one category

Idempotent-ish: fixtures that already declare @page are SKIPPED (they test
paged media / page size themselves). Run from the repo root.
"""
import sys, os, re, subprocess, glob, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CASES = os.path.join(ROOT, "tests", "parity", "cases")
CLI = os.path.join(ROOT, "target", "release", "ironpress")
CSS_PX = 3.125  # device px per CSS px @300dpi

# Global calibration defaults injected into EVERY fixture so all fixtures share
# one well-defined baseline (font + line box). `line-height: normal` and generic
# families are implementation-defined; pinning the bundled face + a numeric
# line-height makes every fixture render identically in ironpress and Chrome.
# Element-level rules in a fixture still override these (so feature tests for a
# specific font/line-height work normally).
GLOBAL_DEFAULTS = "  html { font-family: ParitySans; line-height: 1.5; }\n"

# Map every non-bundled / UA-discretionary family token to a deterministic bundled
# Parity face (applied ONLY inside font-family values). Order matters: the longer
# `sans-serif` is consumed before the `serif` sub-token; quoted/spaced aliases and
# `ParityCustom` (registered as the serif fallback) are normalized too.
FAMILY_MAP = [
    (r"'Parity Sans'|\"Parity Sans\"", "ParitySans"),
    (r"'Parity Serif'|\"Parity Serif\"", "ParitySerif"),
    (r"'Parity Mono'|\"Parity Mono\"", "ParityMono"),
    (r"'ParityCustom'|\"ParityCustom\"|\bParityCustom\b", "ParitySerif"),
    (r"\bsans-serif\b", "ParitySans"),
    (r"(?<!-)\bserif\b", "ParitySerif"),
    (r"\bmonospace\b", "ParityMono"),
]
# categories whose fixtures legitimately control the page themselves
SKIP_CATEGORIES = {"paged-media"}


def content_bbox_dev(png):
    """Return (minx,miny,maxx,maxy) of non-white pixels, or None."""
    from PIL import Image
    im = Image.open(png).convert("L")
    # non-white mask: anything darker than 245
    mask = im.point(lambda v: 255 if v < 245 else 0)
    return mask.getbbox()  # (l,t,r,b) exclusive r/b


def render_bbox(html_path, page_args):
    """Render via ironpress CLI, rasterize @300dpi, return content bbox (dev px)."""
    with tempfile.TemporaryDirectory(dir=os.path.expanduser("~")) as td:
        pdf = os.path.join(td, "o.pdf")
        base = os.path.dirname(os.path.abspath(html_path))
        cmd = [CLI, "--base-path", base] + page_args + [html_path, pdf]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0 or not os.path.exists(pdf):
            raise RuntimeError(f"render failed: {r.stderr[:300]}")
        subprocess.run(["pdftoppm", "-r", "300", "-png", "-f", "1", "-l", "1", pdf,
                        os.path.join(td, "p")], check=True, capture_output=True)
        pngs = sorted(glob.glob(os.path.join(td, "p*.png")))
        if not pngs:
            raise RuntimeError("no raster")
        return content_bbox_dev(pngs[0])


def _inject_after_style(html, block):
    """Insert `block` (already-indented, trailing newline) right after <style>."""
    return re.sub(r"(<style[^>]*>[ \t]*\n)", lambda m: m.group(1) + block, html, count=1)


def _map_family_value(val):
    for pat, repl in FAMILY_MAP:
        val = re.sub(pat, repl, val)
    return val


def transform_text(html):
    """Normalize fonts to bundled Parity faces and inject the global calibration
    defaults. Returns (new_html, changed)."""
    # Rewrite every font-family VALUE (only) to the bundled bare names.
    html = re.sub(
        r"(font-family\s*:)([^;}]*)",
        lambda m: m.group(1) + _map_family_value(m.group(2)),
        html,
    )
    # Inject the shared global defaults once, right after <style> (element-level
    # rules in the fixture override them by specificity / source order).
    html = _inject_after_style(html, GLOBAL_DEFAULTS)
    return html, True


def inject_page(html, w_css, h_css):
    """Insert an @page rule sizing the page to content with margin:0."""
    rule = f"  @page {{ size: {w_css}px {h_css}px; margin: 0; }}\n"
    return _inject_after_style(html, rule)


def process(path):
    cat = os.path.basename(os.path.dirname(path))
    name = os.path.basename(path)
    if cat in SKIP_CATEGORIES:
        return ("skip-cat", name)
    src = open(path, encoding="utf-8").read()
    if re.search(r"@page\b", src):
        return ("skip-haspage", name)

    import math
    new, changed = transform_text(src)
    tmp = path + ".tmp"
    try:
        # PASS 1 — natural content extent on a wide LETTER page (gives the width).
        open(tmp, "w", encoding="utf-8").write(new)
        bb = render_bbox(tmp, ["--page-size", "letter", "--margin", "28.8"])
        if bb is None:
            return ("blank", name)
        minx, miny, maxx, maxy = bb
        # symmetric framing: content sits at (minx,miny); mirror that margin to R/B
        w_css = math.ceil((maxx + minx) / CSS_PX)

        # PASS 2 — re-render at the TARGET width on a tall page so width-dependent
        # wrapping text reflows to its true height. A 1px sentinel block appended at
        # the end of <body> marks the TRUE LAYOUT BOTTOM (the ink bbox alone misses
        # trailing margin/padding below the last painted pixel — e.g. a wrapper's
        # padding-bottom — which still occupies layout height and would otherwise
        # overflow the @page and make Chrome paginate). The sentinel is full-width so
        # its x is ignored; only its y (the layout bottom) is used.
        sentinel = '<div style="height:1px;background:#000"></div>\n</body>'
        new_sent = new.replace("</body>", sentinel, 1) if "</body>" in new else new
        open(tmp, "w", encoding="utf-8").write(inject_page(new_sent, w_css, 4000))
        bb2 = render_bbox(tmp, [])
        if bb2 is None:
            _, _, _, maxy2 = (0, 0, 0, maxy)
        else:
            _, _, _, maxy2 = bb2
        # page height = true layout bottom + a small buffer (absorbs Chrome's @page
        # pt-rounding so content never tips onto a 2nd page). Width stays from pass 1
        # (symmetric); pass-2 x is meaningless (the sentinel spans the full width).
        h_css = math.ceil(maxy2 / CSS_PX) + 4

        final = inject_page(new, w_css, h_css)
        open(tmp, "w", encoding="utf-8").write(final)
        # VALIDATION — render at the final @page and confirm nothing clips.
        bb3 = render_bbox(tmp, [])
        clipped = ""
        if bb3 is not None:
            _, _, mx3, my3 = bb3
            if mx3 >= w_css * CSS_PX - 2 or my3 >= h_css * CSS_PX - 2:
                clipped = f"  !!CLIP cand-extent=({mx3},{my3}) page=({w_css*CSS_PX:.0f},{h_css*CSS_PX:.0f})"
        open(path, "w", encoding="utf-8").write(final)
        status = "clip" if clipped else "ok"
        return (status, f"{name} -> {w_css}x{h_css}px css{clipped}")
    finally:
        if os.path.exists(tmp):
            os.remove(tmp)


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__); return
    targets = []
    if args[0] == "--all":
        targets = sorted(glob.glob(os.path.join(CASES, "*", "*.html")))
    elif args[0] == "--category":
        targets = sorted(glob.glob(os.path.join(CASES, args[1], "*.html")))
    else:
        targets = [os.path.abspath(a) for a in args]

    counts = {}
    for p in targets:
        try:
            status, msg = process(p)
        except Exception as e:
            status, msg = "error", f"{os.path.basename(p)}: {e}"
        counts[status] = counts.get(status, 0) + 1
        if status in ("ok", "error", "blank", "clip"):
            print(f"  [{status}] {msg}")
    print("summary:", counts)
    if counts.get("clip"):
        print("  NOTE: [clip] fixtures need manual page sizing (content reflowed to the page edge).")


if __name__ == "__main__":
    main()
