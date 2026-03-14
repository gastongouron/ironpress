# ironpress

[![Crates.io](https://img.shields.io/crates/v/ironpress.svg)](https://crates.io/crates/ironpress)
[![docs.rs](https://docs.rs/ironpress/badge.svg)](https://docs.rs/ironpress)
[![CI](https://github.com/gastongouron/ironpress/actions/workflows/ci.yml/badge.svg)](https://github.com/gastongouron/ironpress/actions)
[![codecov](https://codecov.io/gh/gastongouron/ironpress/graph/badge.svg?token=w36XIAwRxG)](https://codecov.io/gh/gastongouron/ironpress)

Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.

<p align="center">
  <a href="https://codecov.io/gh/gastongouron/ironpress">
    <img src="https://codecov.io/gh/gastongouron/ironpress/graphs/tree.svg?token=w36XIAwRxG" alt="Coverage grid">
  </a>
</p>

Other Rust PDF crates shell out to headless Chrome or wkhtmltopdf. ironpress does it natively with a built-in layout engine. No C libraries, no binaries to install, just `cargo add ironpress`.

## Quick Start

```rust
use ironpress::html_to_pdf;

let pdf_bytes = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
std::fs::write("output.pdf", pdf_bytes).unwrap();
```

## With Options

```rust
use ironpress::{HtmlConverter, PageSize, Margin};

let pdf = HtmlConverter::new()
    .page_size(PageSize::LETTER)
    .margin(Margin::uniform(54.0))
    .convert("<h1>Custom page</h1>")
    .unwrap();
```

## Markdown to PDF

```rust
let pdf = ironpress::markdown_to_pdf("# Hello\n\n**Bold** and *italic* text.").unwrap();
```

```rust
ironpress::convert_markdown_file("README.md", "readme.pdf").unwrap();
```

Built-in Markdown parser with zero external dependencies. Supports headings, bold, italic, inline code, fenced code blocks, links, images, ordered and unordered lists, blockquotes, and horizontal rules.

## File Conversion

```rust
ironpress::convert_file("input.html", "output.pdf").unwrap();
```

## Supported HTML Elements

| Category | Elements |
|----------|----------|
| Headings | `<h1>` through `<h6>` with default sizes and bold |
| Block containers | `<p>`, `<div>`, `<blockquote>`, `<pre>`, `<figure>`, `<figcaption>`, `<address>` |
| Semantic sections | `<section>`, `<article>`, `<nav>`, `<header>`, `<footer>`, `<main>`, `<aside>`, `<details>`, `<summary>` |
| Inline formatting | `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<small>`, `<sub>`, `<sup>`, `<code>`, `<abbr>`, `<span>` |
| Text decoration | `<del>`, `<s>` (strikethrough), `<ins>` (underline), `<mark>` (highlight) |
| Links | `<a>` with colored underlined text |
| Line breaks | `<br>`, `<hr>` |
| Lists | `<ul>`, `<ol>` with bullets and numbers, `<li>`, `<dl>`, `<dt>`, `<dd>` |
| Tables | `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`, `<td>`, `<th>`, `<caption>` with multi-column layout and cell borders |

## CSS Support

### Inline styles (`style="..."`)

`font-size`, `font-weight`, `font-style`, `color`, `background-color`, `margin`, `padding`, `text-align`, `text-decoration`, `line-height`, `page-break-before`, `page-break-after`

### `<style>` blocks

```html
<style>
  p { color: navy; font-size: 14pt }
  .highlight { background-color: yellow }
  #title { font-size: 24pt }
  h1, h2 { color: darkblue }
</style>
```

Supported selectors: tag names (`p`, `h1`), classes (`.foo`), IDs (`#bar`), combined (`p.foo`), comma-separated (`h1, h2`).

Colors: named colors, `#hex`, `rgb()`. Units: `px`, `pt`, `em`.

## Security

HTML is sanitized by default before conversion:

- `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>` tags are stripped
- `<style>` tags are preserved but dangerous CSS (`@import`, external `url()`, `expression()`) is removed
- Event handlers (`onclick`, `onload`, etc.) are removed
- `javascript:` URLs are neutralized
- Input size and nesting depth are limited

Sanitization can be disabled with `.sanitize(false)` if you trust the input.

## How It Works

```
Input -> Sanitize -> Parse (html5ever) -> Extract <style> -> Style cascade -> Layout engine -> PDF
```

1. **Sanitize** the input HTML to remove dangerous elements
2. **Parse** HTML into a DOM tree using html5ever, extracting `<style>` blocks
3. **Resolve styles** by cascading tag defaults, then stylesheet rules, then inline CSS
4. **Layout** elements with text wrapping, page breaks, tables, lists, and box model
5. **Render** to PDF using built-in Helvetica fonts (no font embedding needed)

For Markdown input, an additional step converts Markdown to HTML first using the built-in parser.

## License

MIT
