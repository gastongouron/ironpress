
<div align="center">

<img width="188" alt="4" src="https://github.com/user-attachments/assets/e8b569e6-e74c-4c0f-9e84-05cf37fae3ae" />

# Ironpress

Pure rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.


[![Crates.io](https://img.shields.io/crates/v/ironpress.svg)](https://crates.io/crates/ironpress)
[![PyPI](https://img.shields.io/pypi/v/ironpress.svg)](https://pypi.org/project/ironpress/)
[![Gem](https://img.shields.io/gem/v/ironpress.svg)](https://rubygems.org/gems/ironpress)
[![npm](https://img.shields.io/npm/v/ironpress.svg)](https://www.npmjs.com/package/ironpress)
[![docs.rs](https://docs.rs/ironpress/badge.svg)](https://docs.rs/ironpress)
[![CI](https://github.com/gastongouron/ironpress/actions/workflows/ci.yml/badge.svg)](https://github.com/gastongouron/ironpress/actions)
[![codecov](https://codecov.io/gh/gastongouron/ironpress/branch/main/graph/badge.svg?token=w36XIAwRxG)](https://codecov.io/gh/gastongouron/ironpress)
[![deps.rs](https://deps.rs/repo/github/gastongouron/ironpress/status.svg)](https://deps.rs/repo/github/gastongouron/ironpress)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Downloads](https://img.shields.io/crates/d/ironpress.svg)](https://crates.io/crates/ironpress)
[![WASM](https://img.shields.io/badge/wasm-ready-blueviolet.svg)](../../wiki/WASM-Playground)
[![Playground](https://img.shields.io/badge/try_it-playground-blueviolet.svg)](https://gastongouron.github.io/ironpress/)
[![Parity](https://img.shields.io/badge/parity-dashboard-ff69b4.svg)](https://gastongouron.github.io/ironpress/parity/)

**[Try it in your browser](https://gastongouron.github.io/ironpress/)** | **[Parity dashboard](https://gastongouron.github.io/ironpress/parity/)** | **[Wiki](../../wiki)**

</div>


## Performance

<!-- AUTO:BENCH - updated by CI -->
| Document | Time | Pages/sec |
|----------|------|-----------|
| Simple HTML (`<h1>` + `<p>`) | **16 us** | 62,500 |
| Styled HTML (CSS, lists, links) | **71 us** | 14,000 |
| Markdown (headings, code, lists) | **141 us** | 7,000 |
| Table (5 rows, styled headers) | **341 us** | 2,900 |
| Full report (tables, flex, progress bars) | **587 us** | 1,700 |
<!-- /AUTO:BENCH -->

Chrome headless takes ~2,500 ms per page. **ironpress is 4,000x faster**.

## Quick start

```rust
use ironpress::html_to_pdf;

let pdf = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
std::fs::write("output.pdf", pdf).unwrap();
```

```rust
let pdf = ironpress::markdown_to_pdf("# Hello\n\nWorld").unwrap();
```

## CLI

```bash
cargo install ironpress

ironpress input.html output.pdf
ironpress document.md output.pdf
ironpress --page-size letter --landscape --margin 54 input.html output.pdf
ironpress --header "Report" --footer "Page {page} of {pages}" input.html output.pdf
echo '<h1>Hello</h1>' | ironpress --stdin output.pdf
```

## Builder API

```rust
use ironpress::{HtmlConverter, PageSize, Margin};

let pdf = HtmlConverter::new()
    .page_size(PageSize::LETTER)
    .margin(Margin::uniform(54.0))
    .header("My Document")
    .footer("Page {page} of {pages}")
    .convert("<h1>Custom page</h1>")
    .unwrap();
```

## Features at a glance

| Area | Highlights | Details |
|------|-----------|---------|
| **HTML** | 50+ elements: headings, tables, lists, forms, media, `<img>`, inline `<svg>` | [Layout Engine](../../wiki/Layout-Engine) |
| **CSS** | Flexbox, grid, multi-column, `calc()`, variables, `@media`, `@page`, `@font-face` | [CSS Support](../../wiki/CSS-Support) |
| **Fonts** | Base-14 PDF fonts, custom TTF embedding with subsetting, system font discovery, Unicode/CJK fallback | [Font System](../../wiki/Font-System) |
| **Math** | LaTeX via `$...$` / `$$...$$`: fractions, roots, matrices, Greek, operators | [Math Engine](../../wiki/Math-Engine) |
| **SVG** | Vector rendering: path, shapes, gradients, transforms, clip paths, `viewBox` | [Layout Engine](../../wiki/Layout-Engine) |
| **Images** | JPEG + PNG, data URIs, local files, remote URLs (`remote` feature) | [Architecture](../../wiki/Architecture) |
| **PDF** | PDF 1.4, bookmarks, link annotations, headers/footers, gradients, streaming output | [PDF Rendering](../../wiki/PDF-Rendering) |
| **WASM** | `npm install ironpress` - runs 100% client-side in the browser | [WASM & Playground](../../wiki/WASM-Playground) |
| **Testing** | 2200+ unit tests, property-based tests, 6 fuzz targets, parity dashboard | [Testing Strategy](../../wiki/Testing-Strategy) |

## Custom fonts

```rust
let pdf = HtmlConverter::new()
    .add_font("Inter", std::fs::read("Inter.ttf").unwrap())
    .convert(r#"<p style="font-family: Inter">Shaped with HarfBuzz</p>"#)
    .unwrap();
```

Fonts are shaped with [rustybuzz](https://crates.io/crates/rustybuzz), subset to used glyphs only, and embedded as CIDFontType2. Characters outside WinAnsi (CJK, Arabic, emoji) are rendered via automatic Unicode font fallback. See [Font System](../../wiki/Font-System).

## Math

```markdown
The equation $E = mc^2$ is famous.

$$\sum_{k=1}^{n} k = \frac{n(n+1)}{2}$$
```

Full LaTeX support: fractions, roots, matrices, Greek letters, operators, delimiters, accents. See [Math Engine](../../wiki/Math-Engine).

## Python / Ruby

```bash
pip install ironpress
```

```python
import ironpress
pdf = ironpress.html_to_pdf("<h1>Hello</h1>")
```

```bash
gem install ironpress
```

```ruby
require "ironpress"
pdf = Ironpress.html_to_pdf("<h1>Hello</h1>")
```

## WASM

```bash
npm install ironpress
```

```javascript
import init, { htmlToPdf, markdownToPdf } from 'ironpress';
await init();

const pdf = htmlToPdf('<h1>Hello</h1>');
const blob = new Blob([pdf], { type: 'application/pdf' });
```

See [WASM & Playground](../../wiki/WASM-Playground).

## Security

HTML is sanitized by default: `<script>`, `<iframe>`, event handlers, and `javascript:` URLs are stripped. Resources are sandboxed (local-only by default, 10 MB cap). SVG sanitizer strips dangerous elements. PNG decompression capped at 50 MB. Disable with `.sanitize(false)` if you trust the input.

## How it works

```
HTML/Markdown → Sanitize → Parse (html5ever) → Style cascade → Layout engine → PDF 1.4
```

See [Architecture](../../wiki/Architecture) for the full pipeline.

## Visual parity harness

`tests/parity/` is a granular Chrome-parity test suite: one isolated fixture per
CSS feature/value (plus feature-interaction fixtures), each rendered by ironpress
**and** by headless Chrome, then compared with a perceptual **SSIM** metric
(`image-compare`, anti-aliasing-robust) at 300 DPI. It tracks a per-feature /
per-category / overall implementation score and pinpoints whether a failure is
**REAL** (the feature itself) or **CONFOUNDED** (a substrate it depends on, via
"probe" fixtures).

```bash
scripts/parity.sh                 # run the engine: cargo test --test feature_parity
scripts/parity-gen-refs.sh --force  # regenerate the committed Chrome references (needs Chromium)
```

- **Run it:** `cargo test --test feature_parity` — renders each fixture in-process,
  rasterizes via `pdftoppm`, diffs against the committed reference, and writes
  `tests/parity/report.json` + `REPORT.md` + the per-category visual reports.
- **Read it:** open `tests/parity/reports/index.html` — it summarizes every
  category and links a page per category, grouped by feature (anchored, scored),
  each fixture showing **Chrome ref ∣ ironpress ∣ SSIM diff**. (GitHub serves
  committed HTML as source; view the rendered report via the `htmlpreview` link the
  PR bot posts, the GitHub Pages site, or locally.)
- **References are committed** (Git LFS) so the suite runs without Chrome. When you
  change a fixture you must regenerate its reference: `scripts/parity-gen-refs.sh
  --force` then commit `tests/parity/{refs,refs.lock}`. CI's freshness gate
  (`refs.lock` = sha256 of every fixture) fails the build if a fixture changed
  without its reference being regenerated.
- **CI:** `.github/workflows/parity.yml` is the deterministic gate (no Chrome,
  runs against committed refs + freshness check); `parity-visuals.yml` regenerates
  refs with a pinned Chromium on PRs and posts a comment linking the rendered
  report index.
- **Git LFS** stores the reference/candidate/diff PNGs and bundled fonts — run
  `git lfs install` once after cloning.

## License

MIT
