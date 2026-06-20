# ironpress Feature-Parity Engine

This directory holds the feature-parity test suite. Each fixture isolates **one**
CSS/HTML feature (or one interaction between two features), is rendered
**in-process** through the `ironpress` library at Chrome-matching page geometry,
and is diffed against a **committed Chrome reference raster**. The engine scores
parity per feature / per category / overall, writes a machine scorecard
(`report.json`) and a human one (`REPORT.md`), and gates CI on regressions.

Chrome is **never** run at test time — references are pre-generated once and
committed.

## Layout

```
tests/
  feature_parity.rs              # integration entry: `cargo test --test feature_parity`
  parity_support/mod.rs          # the engine implementation
  parity/
    cases/<category>/<id>.html   # standalone fixtures (committed)
    refs/<category>/<id>.png     # Chrome references @150 DPI (committed)
    manifest/<category>.json     # manifest fragment: JSON array of entries (committed)
    report.json                  # machine scorecard / regression baseline (committed)
    REPORT.md                    # human scorecard (committed)
    diffs/<category>/<id>.png    # failure overlays (gitignored)
scripts/
  parity-gen-refs.sh             # one-time Chrome reference generator
  parity.sh                      # convenience runner
```

## Running

```bash
scripts/parity.sh
# or directly:
cargo test --test feature_parity -- --nocapture
```

The engine always rewrites `report.json` + `REPORT.md`, then fails the test only
on a **regression**: an overall-score drop beyond a small epsilon, or a fixture
that was `PASS` in the committed `report.json` and is now `FAIL`. New fixtures and
`UNKNOWN` fixtures (no reference) never fail the build. If `pdftoppm` is missing,
every fixture is `UNKNOWN` and the run still passes.

## Adding a fixture (one feature per file)

1. Write a **standalone, deterministic** HTML document at
   `tests/parity/cases/<category>/<id>.html`:
   - Self-contained: **no external resources, no web fonts, no network.**
   - Use only generic font families (`serif` / `sans-serif` / `monospace`) with
     explicit `px`/`pt` sizes and explicit colors so both engines agree on
     metrics.
   - Isolate **one** feature/value. Keep the region small and bounded with solid
     fills and `>=2px` borders so geometric differences show up as pixel diffs.
     Prefer boxes over text for pure-layout features (text shaping differs between
     engines); reserve text for `typography` / `inline` categories and keep it
     short.
   - Reset default margins: `* { margin:0; box-sizing:border-box }` unless the
     fixture specifically tests margins / box-model defaults.
   - **Do NOT declare `@page { size / margin }`** — it would override the page
     geometry on the ironpress side only and desynchronize candidate vs.
     reference. The engine rejects fixtures containing `@page`.
   - Keep all content within **one** US Letter page.

2. Add an entry to `tests/parity/manifest/<category>.json` (a JSON array). The
   filename stem must equal each entry's `category`, and `id` must equal the
   fixture filename stem. Minimal entry:

   ```json
   {
     "id": "flexbox-justify-content-space-between",
     "category": "flexbox",
     "feature": "justify-content",
     "subfeature": "space-between",
     "description": "Three boxes spaced space-between in a row flex container.",
     "file": "cases/flexbox/flexbox-justify-content-space-between.html"
   }
   ```

   Optional fields: `weight` (default 1.0), `pass_threshold_pct` (default 2.0),
   `partial_threshold_pct` (default 10.0), `sanitize` (default true). For an
   **interaction** fixture add `interaction_of` (>=2 categories) and `base_ids`
   (the single-feature fixture ids it combines) so the report can classify the
   failure as GENUINE vs. DERIVATIVE.

3. Generate the reference (one time, requires chromium + poppler):

   ```bash
   scripts/parity-gen-refs.sh <category>     # or no arg for all missing refs
   ```

   Commit the resulting `tests/parity/refs/<category>/<id>.png`.

4. Run the engine to update the scorecard and commit `report.json` + `REPORT.md`:

   ```bash
   scripts/parity.sh
   ```

## Reading the scorecard

`REPORT.md` leads with a **Regressions / Failures** table that names the exact
feature/subfeature (or interaction) that is wrong, followed by a per-category
coverage table and a category -> feature -> fixture detail tree, and finally the
UNKNOWN (untested) list.

`report.json` is the machine-readable baseline used by the regression gate. Its
`env` block records DPI and tolerances for provenance.

### Status meaning

| status   | meaning                                                              |
|----------|---------------------------------------------------------------------|
| PASS     | `diff_pct <= pass_threshold_pct` (default 2%)                        |
| PARTIAL  | `diff_pct <= partial_threshold_pct` (default 10%)                    |
| FAIL     | above partial, or render error / malformed PDF / pdftoppm failure   |
| UNKNOWN  | no committed reference (excluded from the score, never gates)        |

### Interpreting the percentage

The overall / per-category / per-feature score is a **weighted pass-rate** over
fixtures that have a reference (`PASS=1.0`, `PARTIAL=0.5`, `FAIL=0`, `UNKNOWN`
excluded). It is the percentage of **written** fixtures that pass — read it
together with **Scored coverage** (how many fixtures actually have a reference) so
a high score over a tiny tested subset is not mistaken for broad parity.

## How diffing works

For each fixture with a reference: the candidate PDF is rasterized with
`pdftoppm -r 150`, both candidate and reference are cropped to their non-white
content bounding box, the candidate is resized to the reference dimensions, and a
per-pixel diff (per-channel tolerance) yields `diff_pct`. On any non-PASS result a
magenta-highlighted overlay is written to `tests/parity/diffs/<category>/<id>.png`
(gitignored) for inspection.
