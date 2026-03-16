# ironpress

[![Crates.io](https://img.shields.io/crates/v/ironpress.svg)](https://crates.io/crates/ironpress)
[![docs.rs](https://docs.rs/ironpress/badge.svg)](https://docs.rs/ironpress)
[![CI](https://github.com/gastongouron/ironpress/actions/workflows/ci.yml/badge.svg)](https://github.com/gastongouron/ironpress/actions)
[![codecov](https://codecov.io/gh/gastongouron/ironpress/branch/main/graph/badge.svg?token=w36XIAwRxG)](https://codecov.io/gh/gastongouron/ironpress)

Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.

<p align="center">
  <a href="https://codecov.io/gh/gastongouron/ironpress">
    <img src="https://codecov.io/gh/gastongouron/ironpress/graphs/sunburst.svg?token=w36XIAwRxG" alt="Coverage grid">
  </a>
</p>

Other Rust PDF crates shell out to headless Chrome or wkhtmltopdf. ironpress does it natively with a built-in layout engine. No C libraries, no binaries to install, just `cargo add ironpress`.

<img width="1469" height="925" alt="Image" src="https://github.com/user-attachments/assets/db18a4cc-72a3-4ccd-8cea-78cf07c40f7a" />

## Table of Contents

- [Quick Start](#quick-start)
- [API Reference](#api-reference)
- [Markdown to PDF](#markdown-to-pdf)
- [HTML Elements](#html-elements)
- [CSS Support](#css-support)
- [Images](#images)
- [SVG](#svg)
- [Tables](#tables)
- [Fonts](#fonts)
- [Streaming Output](#streaming-output)
- [Async API](#async-api)
- [Security](#security)
- [How It Works](#how-it-works)
- [WASM](#wasm)
- [Testing](#testing)
- [License](#license)

## Quick Start

```rust
use ironpress::html_to_pdf;

let pdf_bytes = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
std::fs::write("output.pdf", pdf_bytes).unwrap();
```

## API Reference

### One-liner functions

```rust
// HTML string to PDF bytes
let pdf = ironpress::html_to_pdf("<h1>Title</h1><p>Content</p>").unwrap();

// Markdown string to PDF bytes
let pdf = ironpress::markdown_to_pdf("# Title\n\nContent").unwrap();

// HTML file to PDF file
ironpress::convert_file("input.html", "output.pdf").unwrap();

// Markdown file to PDF file
ironpress::convert_markdown_file("input.md", "output.pdf").unwrap();
```

### Builder API

```rust
use ironpress::{HtmlConverter, PageSize, Margin};

let pdf = HtmlConverter::new()
    .page_size(PageSize::LETTER)        // default: A4
    .margin(Margin::uniform(54.0))      // default: 72pt (1 inch)
    .sanitize(false)                    // default: true
    .convert("<h1>Custom page</h1>")
    .unwrap();
```

### Custom fonts

```rust
use ironpress::HtmlConverter;

let ttf_data = std::fs::read("fonts/MyFont.ttf").unwrap();
let pdf = HtmlConverter::new()
    .add_font("MyFont", ttf_data)
    .convert(r#"<p style="font-family: MyFont">Custom font text</p>"#)
    .unwrap();
```

### Page sizes

```rust
use ironpress::PageSize;

PageSize::A4         // 595.28 x 841.89 pt (default)
PageSize::LETTER     // 612.0 x 792.0 pt
PageSize::LEGAL      // 612.0 x 1008.0 pt
PageSize::new(width_pt, height_pt)  // custom
```

### Margins

```rust
use ironpress::Margin;

Margin::default()                    // 72pt on all sides (1 inch)
Margin::uniform(54.0)               // same value on all sides
Margin::new(top, right, bottom, left)  // individual values in pt
```

## Markdown to PDF

Built-in Markdown parser with zero external dependencies.

```rust
let pdf = ironpress::markdown_to_pdf(r#"
# Project Title

Some **bold** and *italic* text with `inline code`.

## Features

- Item one
- Item two
- Item three

1. First
2. Second

> A wise quote

---

[Link text](https://example.com)
"#).unwrap();
```

Supported Markdown syntax: headings (`#` to `######`), bold (`**`), italic (`*`), bold+italic (`***`), inline code, fenced code blocks, links, images, unordered lists (`-`, `*`, `+`), ordered lists, blockquotes, and horizontal rules.

## HTML Elements

| Category | Elements |
|----------|----------|
| Headings | `<h1>` through `<h6>` with default sizes and bold |
| Block containers | `<p>`, `<div>`, `<blockquote>`, `<pre>`, `<figure>`, `<figcaption>`, `<address>` |
| Semantic sections | `<section>`, `<article>`, `<nav>`, `<header>`, `<footer>`, `<main>`, `<aside>`, `<details>`, `<summary>` |
| Inline formatting | `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<small>`, `<sub>`, `<sup>`, `<code>`, `<abbr>`, `<span>` |
| Text decoration | `<del>`, `<s>` (strikethrough), `<ins>` (underline), `<mark>` (highlight) |
| Links | `<a>` with clickable PDF link annotations |
| Images | `<img>` with JPEG and PNG support (data URIs and local files) |
| SVG | Inline `<svg>` with `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`, `<path>`, `<g>`, transforms, viewBox |
| Line breaks | `<br>`, `<hr>` |
| Lists | `<ul>`, `<ol>` with nested support, `<li>`, `<dl>`, `<dt>`, `<dd>` |
| Tables | `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`, `<td>`, `<th>`, `<caption>` with colspan, rowspan, auto-sized columns, and cell borders |

## CSS Support

### Properties

| Category | Properties |
|----------|-----------|
| Typography | `font-size`, `font-weight`, `font-style`, `font-family`, `letter-spacing`, `word-spacing`, `text-indent`, `text-transform`, `white-space`, `vertical-align`, `text-overflow` |
| Colors | `color`, `background-color`, `opacity` |
| Box model | `margin` (including `auto`), `padding`, `border`, `border-top/right/bottom/left`, `border-width`, `border-color`, `border-radius`, `outline`, `outline-width`, `outline-color`, `box-sizing`, `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` |
| Layout | `text-align` (left, center, right, justify), `line-height`, `display` (none, block, inline, flex, grid), `float` (left, right), `clear`, `position` (static, relative, absolute), `z-index` |
| Flexbox | `flex-direction`, `justify-content`, `align-items`, `flex-wrap`, `gap` |
| Grid | `grid-template-columns` (fixed, `fr`, `auto`), `grid-gap` |
| Positioning | `top`, `left`, `z-index` |
| Visual effects | `box-shadow`, `transform` (rotate, scale, translate), `overflow` (visible, hidden), `visibility` |
| Backgrounds | `background-color`, `background-position`, `background-size`, `background-repeat`, `linear-gradient()`, `radial-gradient()` |
| Decoration | `text-decoration` (underline, line-through) |
| Lists | `list-style-type` (disc, circle, square, decimal, lower-alpha, upper-alpha, lower-roman, upper-roman, none), `list-style-position` (inside, outside) |
| Tables | `border-collapse`, `border-spacing` |
| Counters | `counter-reset`, `counter-increment`, `content: counter()` |
| Pseudo-elements | `::before`, `::after` with `content` property |
| Custom properties | `--my-var: value`, `var(--my-var)`, `var(--my-var, fallback)` |
| Functions | `calc()` (with `+`, `-`, `*`, `/` and mixed units) |
| Page control | `page-break-before`, `page-break-after`, `@page` (size, margin) |

All shorthand properties are supported. Margin and padding accept 1, 2, 3, or 4 values. Border accepts `width style color` shorthand.

### `<style>` blocks

```html
<style>
  p { color: navy; font-size: 14pt }
  .highlight { background-color: yellow; font-weight: bold }
  #title { font-size: 24pt }
  h1, h2 { color: darkblue }

  @media print {
    .screen-only { display: none }
  }
</style>
```

### Selectors

| Type | Example |
|------|---------|
| Tag | `p`, `h1`, `div` |
| Class | `.highlight`, `.intro` |
| ID | `#title`, `#nav` |
| Combined | `p.highlight`, `div#main` |
| Comma-separated | `h1, h2, h3` |
| Descendant | `div p`, `article h2` |
| Child | `div > p`, `ul > li` |
| Adjacent sibling | `h1 + p` |
| General sibling | `h1 ~ p` |
| Attribute | `[href]`, `[type="text"]` |
| Pseudo-class | `:first-child`, `:last-child`, `:nth-child()`, `:not()` |
| Pseudo-element | `::before`, `::after` |

### Values

| Type | Examples |
|------|---------|
| Colors | `red`, `navy`, `darkblue`, `#f00`, `#ff0000`, `rgb(255, 0, 0)` |
| Units | `12pt`, `16px`, `1.5em`, `50%`, `2rem`, `10vw`, `5vh` |
| Functions | `calc(100% - 20pt)`, `var(--my-color)`, `var(--size, 12pt)` |
| Keywords | `bold`, `italic`, `center`, `justify`, `none`, `inherit`, `initial`, `unset` |

### Media queries

`@media print` rules are applied (since PDF is print output). `@media screen` rules are ignored.

### `@page` rule

Control page size and margins from CSS:

```html
<style>
  @page { size: letter landscape; margin: 0.5in; }
</style>
```

Supported values: `A4`, `letter`, `legal`, `landscape`, custom dimensions (`210mm 297mm`), and individual margins.

## Images

JPEG and PNG images are supported via data URIs and local file paths.

```html
<!-- Data URI -->
<img src="data:image/jpeg;base64,/9j/4AAQ..." width="200" height="150">

<!-- Local file -->
<img src="photo.jpg" width="300" height="200">
```

Images are embedded directly in the PDF. JPEG uses DCTDecode, PNG uses FlateDecode with PNG predictors. Width and height attributes are converted from px to pt.

## SVG

Inline SVG elements are rendered as vector graphics directly in the PDF (not rasterized):

```html
<svg width="200" height="200" viewBox="0 0 100 100">
  <rect x="10" y="10" width="80" height="80" fill="#e74c3c" stroke="#333" stroke-width="2"/>
  <circle cx="50" cy="50" r="30" fill="#3498db"/>
  <path d="M 20 80 L 50 20 L 80 80 Z" fill="#2ecc71"/>
  <g transform="translate(50, 50) rotate(45)">
    <rect x="-10" y="-10" width="20" height="20" fill="#f39c12"/>
  </g>
</svg>
```

Supported elements: `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`, `<path>` (full path command set: M, L, H, V, C, S, Q, T, Z with relative variants), `<g>` groups with `transform` (translate, scale, rotate, matrix), and `viewBox` scaling.

SVG content is automatically sanitized: `<script>`, `<foreignObject>`, `<use>`, `<image>`, and event handlers are stripped.

## Tables

Full table support with sections, spanning, auto-sized columns, and styling.

```html
<table>
  <thead>
    <tr><th>Name</th><th>Role</th><th>Status</th></tr>
  </thead>
  <tbody>
    <tr>
      <td rowspan="2">Alice</td>
      <td>Engineer</td>
      <td>Active</td>
    </tr>
    <tr>
      <td colspan="2">On project X</td>
    </tr>
    <tr>
      <td>Bob</td>
      <td>Designer</td>
      <td>Active</td>
    </tr>
  </tbody>
</table>
```

Column widths are automatically calculated based on content. Features: `<thead>`, `<tbody>`, `<tfoot>` sections, `colspan` and `rowspan` attributes, bold headers in `<th>`, cell borders, background colors, and padding.

## Fonts

### Standard fonts

ironpress includes the 14 standard PDF fonts (no embedding required). CSS `font-family` values are mapped to the closest match:

| PDF Font | CSS Values |
|----------|-----------|
| Helvetica | `arial`, `helvetica`, `sans-serif`, `verdana`, `tahoma`, `roboto`, `open sans`, `inter`, `system-ui`, and 20+ more |
| Times-Roman | `serif`, `times new roman`, `georgia`, `garamond`, `palatino`, `merriweather`, `lora`, and 15+ more |
| Courier | `monospace`, `courier new`, `consolas`, `fira code`, `jetbrains mono`, `source code pro`, `menlo`, and 15+ more |

Each family includes regular, bold, italic, and bold-italic variants (12 fonts total).

### Custom fonts (TrueType)

Embed any TTF font for pixel-perfect rendering:

```rust
use ironpress::HtmlConverter;

let font = std::fs::read("fonts/Inter.ttf").unwrap();
let pdf = HtmlConverter::new()
    .add_font("Inter", font)
    .convert(r#"<p style="font-family: Inter">Rendered with Inter</p>"#)
    .unwrap();
```

The TTF parser extracts character metrics for accurate text wrapping and embeds the font directly in the PDF.

### `@font-face`

Load fonts directly from CSS (local files only, remote URLs are blocked for security):

```html
<style>
  @font-face {
    font-family: "MyFont";
    src: url("fonts/MyFont.ttf");
  }
  p { font-family: MyFont; }
</style>
<p>Rendered with MyFont</p>
```

Requires `.base_path()` on the builder so the converter knows where to find font files.

## Streaming Output

Write PDF output directly to any `std::io::Write` implementation instead of allocating a `Vec<u8>`:

```rust
use std::fs::File;

let mut file = File::create("output.pdf").unwrap();
ironpress::html_to_pdf_writer("<h1>Hello</h1>", &mut file).unwrap();
```

Also available on the builder:

```rust
use ironpress::HtmlConverter;
use std::fs::File;

let mut file = File::create("output.pdf").unwrap();
HtmlConverter::new()
    .convert_to_writer("<h1>Hello</h1>", &mut file)
    .unwrap();
```

## Async API

Enable the `async` feature for async file I/O:

```toml
ironpress = { version = "0.9", features = ["async"] }
```

```rust
ironpress::convert_file_async("input.html", "output.pdf").await.unwrap();
ironpress::convert_markdown_file_async("input.md", "output.pdf").await.unwrap();
```

The HTML parsing, layout, and rendering remain synchronous (CPU-bound). Async is used for file reads and writes via tokio.

## Security

HTML is sanitized by default before conversion:

- `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>` tags are stripped
- `<style>` tags are preserved but dangerous CSS (external `url()`, `expression()`) is removed
- `@import` and `@font-face` only load local files (remote URLs are blocked, paths sandboxed in `base_dir`)
- Event handlers (`onclick`, `onload`, etc.) are removed
- `javascript:` URLs are neutralized
- Input size (10 MB) and nesting depth (100 levels) are limited
- SVG sanitizer strips `<script>`, `<foreignObject>`, `<use>`, `<image>`, `<style>`, and event handlers inside `<svg>` blocks
- PNG IDAT accumulation capped at 50 MB to prevent decompression bombs
- CSS `@import` cumulative payload capped at 10 MB
- TTF parser validates font metrics and uses checked arithmetic

Sanitization can be disabled with `.sanitize(false)` if you trust the input.

## How It Works

```mermaid
graph LR
    A[HTML / Markdown] --> B[Sanitize]
    B --> C[Parse<br/>html5ever]
    C --> D[Extract<br/>‹style›]
    D --> E[Style<br/>Cascade]
    E --> F[Layout<br/>Engine]
    F --> G[PDF 1.4]

    style A fill:#3498db,color:#fff,stroke:none
    style G fill:#27ae60,color:#fff,stroke:none
```

1. **Sanitize**:strip dangerous elements (`<script>`, `<iframe>`, event handlers, `javascript:` URLs)
2. **Parse**:build a DOM tree using html5ever, extract `<style>` blocks and `@page`/`@font-face` rules
3. **Style cascade**:resolve tag defaults → `@media print` rules → stylesheet rules → inline CSS, with `inherit`/`initial`/`unset` and CSS variable support
4. **Layout**:text wrapping with Adobe font metrics, flexbox, tables with colspan/rowspan, floats, page breaks, images, SVG, and the full CSS box model
5. **Render**:PDF 1.4 output with native Shading Dictionaries for gradients, per-side borders, border-radius, link annotations, embedded images, and TrueType font embedding

For Markdown input, a built-in parser converts Markdown to HTML first (no external dependencies).

## WASM

ironpress compiles to WebAssembly for browser-side PDF generation:

```bash
cargo build --target wasm32-unknown-unknown --no-default-features
```

No system dependencies, no filesystem access needed in the core pipeline.

## Testing

ironpress uses three layers of testing:

- **Unit tests**: 1500+ tests covering parsing, style computation, layout, and rendering
- **Property-based tests**: [proptest](https://crates.io/crates/proptest) verifies invariants across thousands of random inputs (no panics on arbitrary HTML/CSS/Markdown, valid PDF output, correct page structure)
- **Fuzz targets**: [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) targets for HTML, CSS, Markdown, and the full pipeline (`cargo +nightly fuzz run fuzz_html`)

## License

MIT
