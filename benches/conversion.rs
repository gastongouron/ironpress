use criterion::{Criterion, black_box, criterion_group, criterion_main};

const SIMPLE_HTML: &str = "<h1>Hello</h1><p>World</p>";

const STYLED_HTML: &str = r#"
<style>
  body { font-size: 11pt; }
  h1 { color: navy; border-bottom: 2pt solid #ccc; }
  .box { background-color: #f5f5f5; padding: 8pt; border-radius: 4pt; }
</style>
<h1>Report</h1>
<p>Some <strong>bold</strong> and <em>italic</em> text with <a href="https://example.com">a link</a>.</p>
<div class="box"><p>Styled content with background and border-radius.</p></div>
<ul><li>Item one</li><li>Item two</li><li>Item three</li></ul>
"#;

const TABLE_HTML: &str = r#"
<style>
  table { border-collapse: collapse; width: 100%; }
  td, th { border: 1pt solid #ddd; padding: 6pt; }
  th { background-color: #336699; color: white; }
</style>
<h1>Data Report</h1>
<table>
  <thead><tr><th>Name</th><th>Role</th><th>Department</th><th>Status</th></tr></thead>
  <tbody>
    <tr><td>Alice</td><td>Engineer</td><td>Backend</td><td>Active</td></tr>
    <tr><td>Bob</td><td>Designer</td><td>Frontend</td><td>Active</td></tr>
    <tr><td>Charlie</td><td>Manager</td><td>Product</td><td>On leave</td></tr>
    <tr><td>Diana</td><td>Engineer</td><td>Infra</td><td>Active</td></tr>
    <tr><td>Eve</td><td>Analyst</td><td>Data</td><td>Active</td></tr>
  </tbody>
</table>
"#;

const FULL_DOCUMENT: &str = r#"
<style>
  body { font-size: 11pt; color: #333; }
  h1 { color: navy; border-bottom: 2pt solid #ccc; padding-bottom: 4pt; }
  h2 { color: #336699; margin-top: 12pt; }
  table { border-collapse: collapse; width: 100%; margin: 8pt 0; }
  td, th { border: 1pt solid #ddd; padding: 6pt; }
  th { background-color: #336699; color: white; font-weight: bold; }
  .highlight { background-color: #ffffcc; }
  blockquote { border-left: 3pt solid #ccc; padding-left: 10pt; font-style: italic; color: #666; }
  code { background-color: #f5f5f5; padding: 2pt 4pt; font-size: 10pt; }
  .flex { display: flex; gap: 10pt; }
  .flex > div { flex: 1; padding: 8pt; background-color: #f0f0f0; border-radius: 4pt; }
</style>
<h1>Annual Report 2026</h1>
<p>This is a comprehensive document with <strong>bold</strong>, <em>italic</em>, <code>inline code</code>, and <a href="https://example.com">links</a>.</p>

<h2>Financial Summary</h2>
<table>
  <thead><tr><th>Quarter</th><th>Revenue</th><th>Expenses</th><th>Growth</th></tr></thead>
  <tbody>
    <tr><td>Q1</td><td>$1.2M</td><td>$0.8M</td><td class="highlight">+15%</td></tr>
    <tr><td>Q2</td><td>$1.4M</td><td>$0.9M</td><td class="highlight">+17%</td></tr>
    <tr><td>Q3</td><td>$1.1M</td><td>$0.7M</td><td>-2%</td></tr>
    <tr><td>Q4</td><td>$1.8M</td><td>$1.0M</td><td class="highlight">+22%</td></tr>
  </tbody>
</table>

<h2>Key Metrics</h2>
<div class="flex">
  <div><strong>Users</strong><br>12,450</div>
  <div><strong>Revenue</strong><br>$5.5M</div>
  <div><strong>Growth</strong><br>+13%</div>
</div>

<h2>Team Updates</h2>
<ul>
  <li>Engineering: shipped 42 features</li>
  <li>Design: completed brand refresh</li>
  <li>Sales: exceeded Q4 target by 15%</li>
</ul>
<ol>
  <li>Expand to European market</li>
  <li>Launch mobile application</li>
  <li>Achieve SOC 2 certification</li>
</ol>

<blockquote>Quality is not an act, it is a habit. — Aristotle</blockquote>

<h2>Progress</h2>
<p>Project Alpha: <progress value="85" max="100"></progress></p>
<p>Project Beta: <progress value="40" max="100"></progress></p>

<hr>
<p><small>Confidential — Internal use only — Generated with ironpress</small></p>
"#;

const MARKDOWN: &str = r#"# Project Documentation

## Overview

This is a **comprehensive** project document with *various* formatting.

## Features

- Feature one with `inline code`
- Feature two with **bold emphasis**
- Feature three with [a link](https://example.com)

## Code Example

```rust
fn main() {
    println!("Hello, world!");
}
```

## Roadmap

1. Phase one: Research
2. Phase two: Development
3. Phase three: Launch

> This project represents our commitment to excellence.

---

*Last updated: 2026*
"#;

fn bench_simple(c: &mut Criterion) {
    c.bench_function("simple_html", |b| {
        b.iter(|| ironpress::html_to_pdf(black_box(SIMPLE_HTML)).unwrap())
    });
}

fn bench_styled(c: &mut Criterion) {
    c.bench_function("styled_html", |b| {
        b.iter(|| ironpress::html_to_pdf(black_box(STYLED_HTML)).unwrap())
    });
}

fn bench_table(c: &mut Criterion) {
    c.bench_function("table_html", |b| {
        b.iter(|| ironpress::html_to_pdf(black_box(TABLE_HTML)).unwrap())
    });
}

fn bench_full_document(c: &mut Criterion) {
    c.bench_function("full_document", |b| {
        b.iter(|| ironpress::html_to_pdf(black_box(FULL_DOCUMENT)).unwrap())
    });
}

fn bench_markdown(c: &mut Criterion) {
    c.bench_function("markdown", |b| {
        b.iter(|| ironpress::markdown_to_pdf(black_box(MARKDOWN)).unwrap())
    });
}

fn bench_with_header_footer(c: &mut Criterion) {
    c.bench_function("header_footer", |b| {
        b.iter(|| {
            ironpress::HtmlConverter::new()
                .header("Report Title")
                .footer("Page {page} of {pages}")
                .convert(black_box(FULL_DOCUMENT))
                .unwrap()
        })
    });
}

criterion_group!(
    benches,
    bench_simple,
    bench_styled,
    bench_table,
    bench_full_document,
    bench_markdown,
    bench_with_header_footer,
);
criterion_main!(benches);
