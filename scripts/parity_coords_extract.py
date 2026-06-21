#!/usr/bin/env python3
"""Chrome --print-to-pdf -> coordinate sidecar extractor (Phase 2b).

Composes Chrome's nested `cm` stack to map content-stream `re` rects to absolute,
top-left-origin PDF points (the net px->pt factor is 0.24*3.125 = 0.75), then
classifies each painted rect by its PAINT operator:

  * `f` / `f*` / `F` / `B` / `b` / `B*` / `b*`  -> a FILL box  (CoordSidecar.boxes)
  * `S` / `s`                                    -> a BORDER (centerline) rect
                                                     (CoordSidecar.borders)

Filters out the full-page white background rect(s) and near-white fills (Chrome
paints an opaque white page background + the `html`/`body` background). Keeps
meaningful colored boxes (w,h > ~2pt, fill not ~white). De-duplicates identical
rects (Chrome sometimes repeats the page bg).

Emits a sidecar matching tests/parity_support/verify/coords.rs exactly:
  { "schema":1, "frame":"chrome-ref-pt", "page_pt":[612,792],
    "boxes":[{"role":"fill","rect_pt":[x,y_tl,w,h],"selector":null}, ...],
    "borders":[{"role":"border","rect_pt":[...],"selector":null}, ...],
    "text_runs":[] }

Usage: parity_coords_extract.py <chrome.pdf> <out.json> <label>
Prints one status line: WROTE <label> (Nf,Nb) | SKIP <label> (no colored box).
A SKIP writes NO file (the fixture stays raster-only).
"""
import json
import os
import re
import sys
import zlib

PAGE_H = 792.0
PAGE_W = 612.0

# Full-page white background rect Chrome emits (x~27.75, y~27.75, w~556, h~736).
# We drop any near-white fill regardless of size, plus the giant page rect.
WHITE_THRESHOLD = 0.96  # min channel value to be treated as "white-ish"
MIN_DIM_PT = 2.0        # ignore hairline rects
# A fill whose area covers most of the printable page is the page background.
PAGE_AREA_FRAC = 0.55   # > 55% of printable area => page bg, drop it


def streams(path):
    data = open(path, "rb").read()
    out = []
    for m in re.finditer(rb"stream\r?\n(.*?)\r?\nendstream", data, re.S):
        raw = m.group(1)
        try:
            dec = zlib.decompress(raw)
        except Exception:
            dec = raw
        out.append(dec)
    return out


def mul(m, n):
    a, b, c, d, e, f = m
    A, B, C, D, E, F = n
    return (a * A + b * C, a * B + b * D, c * A + d * C, c * B + d * D,
            e * A + f * C + E, e * B + f * D + F)


def ap(m, x, y):
    a, b, c, d, e, f = m
    return (a * x + c * y + e, b * x + d * y + f)


def find_content(path):
    """The first stream that carries `re` and a `cm`/`rg`/`m` (Chrome's page body)."""
    for s in streams(path):
        if b" re" in s and (b" cm" in s or b" rg" in s):
            return s.decode("latin1")
    return None


def is_whiteish(rgb):
    return rgb is not None and all(c >= WHITE_THRESHOLD for c in rgb)


def extract(path):
    """Return (fills, borders) as lists of (rgb, x, y_tl, w, h) in pt."""
    content = find_content(path)
    if not content:
        return [], []
    toks = re.findall(r"[-\d.]+|[A-Za-z*'\"]+", content)
    I = (1, 0, 0, 1, 0, 0)
    ctm = I
    stack = []
    st = []
    fill = None
    stroke = None
    last_re = None  # the pending `re` rect, mapped to pt corners
    fills = []
    borders = []

    def map_re(x, y, w, h):
        p0 = ap(ctm, x, y)
        p1 = ap(ctm, x + w, y + h)
        x0, x1 = sorted((p0[0], p1[0]))
        y0, y1 = sorted((p0[1], p1[1]))
        return (round(x0, 3), round(PAGE_H - y1, 3), round(x1 - x0, 3), round(y1 - y0, 3))

    for t in toks:
        if re.match(r"^-?\d|^\.|^-\.", t):
            try:
                st.append(float(t))
            except ValueError:
                pass
            continue
        op = t
        if op == "q":
            stack.append(ctm)
        elif op == "Q":
            ctm = stack.pop() if stack else I
        elif op == "cm" and len(st) >= 6:
            ctm = mul(tuple(st[-6:]), ctm)
        elif op == "rg" and len(st) >= 3:
            fill = tuple(round(v, 4) for v in st[-3:])
        elif op == "g" and len(st) >= 1:
            v = round(st[-1], 4)
            fill = (v, v, v)
        elif op == "RG" and len(st) >= 3:
            stroke = tuple(round(v, 4) for v in st[-3:])
        elif op == "G" and len(st) >= 1:
            v = round(st[-1], 4)
            stroke = (v, v, v)
        elif op == "re" and len(st) >= 4:
            last_re = map_re(*st[-4:])
        elif op in ("f", "f*", "F", "B", "b", "B*", "b*"):
            if last_re is not None:
                fills.append((fill,) + last_re)
            # B/b also stroke; record the same rect as a border too.
            if op in ("B", "b", "B*", "b*") and last_re is not None:
                borders.append((stroke,) + last_re)
            last_re = None
        elif op in ("S", "s"):
            if last_re is not None:
                borders.append((stroke,) + last_re)
            last_re = None
        elif op == "n":
            last_re = None  # clip path (`re W n`) — not painted ink.
        st = []
    return fills, borders


# A fill spanning ~the full printable WIDTH is a margin-to-margin background
# container (body/.stage/.wrap with a bg). Its width = (page - 2*margin), which
# differs between ironpress (28.8pt margins, spec 0.4in) and Chrome (rounds to
# ~27.75pt) by exactly 2*(28.8-27.75)=2.1pt — a frame-margin ROUNDING artifact,
# NOT a layout bug. PdfGeometry would wrongly flag that 2.1pt width delta on every
# such container, so drop them from the vector sidecar; the RasterDiff verifier
# (with its calibrated frame offset) owns full-width backgrounds.
PRINTABLE_W = PAGE_W - 2 * 27.75  # ~556.5pt
FULL_WIDTH_FRAC = 0.9


def keep(rect, printable_area):
    """A meaningful, colored, non-page-background rect?"""
    rgb, x, y, w, h = rect
    if w < MIN_DIM_PT or h < MIN_DIM_PT:
        return False
    if is_whiteish(rgb):
        return False
    if (w * h) > PAGE_AREA_FRAC * printable_area:
        return False  # page background fill
    if w >= FULL_WIDTH_FRAC * PRINTABLE_W:
        return False  # full-width frame-dominated background container (raster owns it)
    return True


def dedup(rects):
    seen = set()
    out = []
    for r in rects:
        # key on geometry rounded to 0.1pt (color may legitimately differ for
        # overlapping fill+border, but identical-geometry duplicates are noise).
        key = (round(r[1], 1), round(r[2], 1), round(r[3], 1), round(r[4], 1), r[0])
        if key in seen:
            continue
        seen.add(key)
        out.append(r)
    return out


def main():
    pdf, out_json, label = sys.argv[1], sys.argv[2], sys.argv[3]
    fills_raw, borders_raw = extract(pdf)
    # Printable area ~ (Letter - 2*28.8pt margin). Chrome rounds to 27.75 origin;
    # use the nominal printable rect for the page-bg fraction test.
    printable_area = (PAGE_W - 2 * 27.75) * (PAGE_H - 2 * 27.75)

    fills = dedup([r for r in fills_raw if keep(r, printable_area)])
    borders = dedup([r for r in borders_raw if keep(r, printable_area)])

    if not fills and not borders:
        # No parseable colored geometry: stays raster-only, write NO sidecar.
        if os.path.exists(out_json):
            os.remove(out_json)
        print(f"SKIP {label} (no colored box)")
        return

    def box(role, r):
        _, x, y, w, h = r
        return {"role": role, "rect_pt": [x, y, w, h], "selector": None}

    sidecar = {
        "schema": 1,
        "frame": "chrome-ref-pt",
        "page_pt": [PAGE_W, PAGE_H],
        "boxes": [box("fill", r) for r in fills],
        "borders": [box("border", r) for r in borders],
        "text_runs": [],
    }
    with open(out_json, "w", encoding="utf-8") as fh:
        json.dump(sidecar, fh, indent=2)
        fh.write("\n")
    print(f"WROTE {label} ({len(fills)}f,{len(borders)}b)")


if __name__ == "__main__":
    main()
