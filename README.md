# ironpress

[![Crates.io](https://img.shields.io/crates/v/ironpress.svg)](https://crates.io/crates/ironpress)
[![docs.rs](https://docs.rs/ironpress/badge.svg)](https://docs.rs/ironpress)
[![CI](https://github.com/gastongouron/ironpress/actions/workflows/ci.yml/badge.svg)](https://github.com/gastongouron/ironpress/actions)
[![codecov](https://codecov.io/gh/gastongouron/ironpress/branch/main/graph/badge.svg)](https://codecov.io/gh/gastongouron/ironpress)

Pure Rust HTML/CSS-to-PDF converter — no browser, no external dependencies.

Every existing Rust crate that converts HTML to PDF shells out to headless Chrome or wkhtmltopdf. **ironpress** does it natively with a built-in layout engine, producing valid PDFs from HTML with inline CSS.

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

## File Conversion

```rust
ironpress::convert_file("input.html", "output.pdf").unwrap();
```

## Supported HTML Elements

| Category | Elements |
|----------|----------|
| Headings | `<h1>` - `<h6>` with default sizes and bold |
| Block containers | `<p>`, `<div>`, `<blockquote>`, `<pre>`, `<figure>`, `<figcaption>`, `<address>` |
| Semantic sections | `<section>`, `<article>`, `<nav>`, `<header>`, `<footer>`, `<main>`, `<aside>`, `<details>`, `<summary>` |
| Inline formatting | `<strong>`, `<b>`, `<em>`, `<i>`, `<u>`, `<small>`, `<sub>`, `<sup>`, `<code>`, `<abbr>`, `<span>` |
| Text decoration | `<del>`, `<s>` (strikethrough), `<ins>` (underline), `<mark>` (highlight) |
| Links | `<a>` with colored underlined text |
| Line breaks | `<br>`, `<hr>` |
| Lists | `<ul>`, `<ol>`, `<li>`, `<dl>`, `<dt>`, `<dd>` |
| Tables | `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`, `<td>`, `<th>`, `<caption>` |
| Media | `<img>` (placeholder) |

## Supported CSS Properties (inline `style="..."`)

`font-size`, `font-weight`, `font-style`, `color`, `background-color`, `margin`, `padding`, `text-align`, `text-decoration`, `line-height`, `page-break-before`, `page-break-after`

Colors: named colors, `#hex`, `rgb()`.

## Security

HTML is sanitized by default before conversion:

- `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>` tags are stripped
- Event handlers (`onclick`, `onload`, etc.) are removed
- `javascript:` URLs are neutralized
- Input size and nesting depth are limited

Sanitization can be disabled with `.sanitize(false)` if you trust the input.

## How It Works

```
HTML string → Parse (html5ever) → Style resolution → Layout engine → PDF generation
```

1. **Parse** HTML into a DOM tree using html5ever
2. **Resolve styles** by merging default tag styles with inline CSS
3. **Layout** elements with text wrapping, page breaks, and box model
4. **Render** to PDF using built-in Helvetica fonts (no font embedding needed)

## License

MIT
