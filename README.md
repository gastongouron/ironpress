# ironpress

[![Crates.io](https://img.shields.io/crates/v/ironpress.svg)](https://crates.io/crates/ironpress)
[![docs.rs](https://docs.rs/ironpress/badge.svg)](https://docs.rs/ironpress)
[![CI](https://github.com/gastongouron/ironpress/actions/workflows/ci.yml/badge.svg)](https://github.com/gastongouron/ironpress/actions)
[![codecov](https://codecov.io/gh/gastongouron/ironpress/graph/badge.svg?token=w36XIAwRxG)](https://codecov.io/gh/gastongouron/ironpress)

Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.

<p align="center">
  <a href="https://codecov.io/gh/gastongouron/ironpress">
    <img src="https://codecov.io/gh/gastongouron/ironpress/graphs/sunburst.svg?token=w36XIAwRxG" alt="Coverage grid">
  </a>
</p>

Other Rust PDF crates shell out to headless Chrome or wkhtmltopdf. ironpress does it natively with a built-in layout engine. No C libraries, no binaries to install, just `cargo add ironpress`.

## Table of Contents

- [Quick Start](#quick-start)
- [API Reference](#api-reference)
- [Markdown to PDF](#markdown-to-pdf)
- [HTML Elements](#html-elements)
- [CSS Support](#css-support)
- [Images](#images)
- [Tables](#tables)
- [Fonts](#fonts)
- [Security](#security)
- [How It Works](#how-it-works)
- [Roadmap](#roadmap)
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

```
fn main() {
    println!("hello");
}
```

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
| Line breaks | `<br>`, `<hr>` |
| Lists | `<ul>`, `<ol>` with nested support, `<li>`, `<dl>`, `<dt>`, `<dd>` |
| Tables | `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`, `<td>`, `<th>`, `<caption>` with colspan, rowspan, and cell borders |

## CSS Support

### Properties

| Category | Properties |
|----------|-----------|
| Typography | `font-size`, `font-weight`, `font-style`, `font-family` |
| Colors | `color`, `background-color` |
| Box model | `margin`, `padding`, `border`, `border-width`, `border-color` |
| Layout | `text-align`, `line-height`, `display` |
| Decoration | `text-decoration` (underline, line-through) |
| Page control | `page-break-before`, `page-break-after` |

All properties support shorthand notation. Margin and padding accept 1, 2, 3, or 4 values. Border accepts `width style color` shorthand.

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

### Values

| Type | Examples |
|------|---------|
| Colors | `red`, `navy`, `darkblue`, `#f00`, `#ff0000`, `rgb(255, 0, 0)` |
| Units | `12pt`, `16px`, `1.5em` |
| Keywords | `bold`, `italic`, `center`, `none` |

### Media queries

`@media print` rules are applied (since PDF is print output). `@media screen` rules are ignored.

## Images

JPEG and PNG images are supported via data URIs and local file paths.

```html
<!-- Data URI -->
<img src="data:image/jpeg;base64,/9j/4AAQ..." width="200" height="150">

<!-- Local file -->
<img src="photo.jpg" width="300" height="200">
```

Images are embedded directly in the PDF. JPEG uses DCTDecode, PNG uses FlateDecode with PNG predictors. Width and height attributes are converted from px to pt.

## Tables

Full table support with sections, spanning, and styling.

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

Features: `<thead>`, `<tbody>`, `<tfoot>` sections, `colspan` and `rowspan` attributes, bold headers in `<th>`, cell borders, background colors, and padding.

## Fonts

ironpress uses the 14 standard PDF fonts (no font embedding required). CSS `font-family` values are mapped to the closest match:

| PDF Font | CSS Values |
|----------|-----------|
| Helvetica | `arial`, `helvetica`, `sans-serif`, `verdana`, `tahoma`, `roboto`, `open sans`, `inter`, `system-ui`, and more |
| Times-Roman | `serif`, `times new roman`, `georgia`, `garamond`, `palatino`, `merriweather`, `lora`, and more |
| Courier | `monospace`, `courier new`, `consolas`, `fira code`, `jetbrains mono`, `source code pro`, `menlo`, and more |

Each family includes regular, bold, italic, and bold-italic variants (12 fonts total). Unknown font names default to Helvetica.

## Security

HTML is sanitized by default before conversion:

- `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>` tags are stripped
- `<style>` tags are preserved but dangerous CSS (`@import`, external `url()`, `expression()`) is removed
- Event handlers (`onclick`, `onload`, etc.) are removed
- `javascript:` URLs are neutralized
- Input size (10 MB) and nesting depth (100 levels) are limited

Sanitization can be disabled with `.sanitize(false)` if you trust the input.

## How It Works

```
Input --> Sanitize --> Parse (html5ever) --> Extract <style> --> Style cascade --> Layout engine --> PDF
```

1. **Sanitize** the input HTML to remove dangerous elements
2. **Parse** HTML into a DOM tree using html5ever, extracting `<style>` blocks
3. **Resolve styles** by cascading: tag defaults, then `@media print` rules, then stylesheet rules, then inline CSS
4. **Layout** elements with text wrapping, page breaks, tables, lists, images, and the CSS box model
5. **Render** to PDF 1.4 with text, graphics, link annotations, and embedded images

For Markdown input, a built-in parser converts Markdown to HTML first (no external dependencies).

## Roadmap

### v0.5 -- New input formats

- [ ] Plain text (TXT) to PDF
- [ ] CSV to PDF (auto-formatted tables)
- [ ] PNG/JPEG to PDF (full-page image conversion)
- [ ] XML to PDF

### v0.6 -- Vector graphics and e-books

- [ ] SVG rendering (paths, shapes, text, basic CSS)
- [ ] EPUB to PDF

### v0.7 -- Office documents

- [ ] DOCX to PDF
- [ ] XLSX to PDF (spreadsheet tables)

### Planned improvements

- [ ] Remote image loading via URL (behind a feature flag)
- [ ] TrueType/OpenType font embedding
- [ ] CSS `float` and `position` properties
- [ ] Advanced CSS selectors (descendant, child, attribute)
- [ ] Table auto-sizing based on content width
- [ ] Hyphenation and text justification

## License

MIT
