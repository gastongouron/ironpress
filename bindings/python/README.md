# ironpress for Python

Generate PDF bytes from HTML or Markdown with a native Rust renderer. No
browser process or system PDF dependency is required.

[Read the complete Python getting-started guide](https://gastongouron.github.io/ironpress/get-started/python/).

## Requirements

- CPython 3.9 or later
- Linux, macOS, or Windows

Published ABI3 wheels do not require a Rust toolchain.

## Install

```bash
python -m pip install ironpress
```

## Create your first PDF

```python
from pathlib import Path
import ironpress

pdf = ironpress.html_to_pdf("<h1>Hello from Python</h1>")
Path("output.pdf").write_bytes(pdf)
```

The result is a Python `bytes` value. Write it to a file, return it from a web
response, or store it in your application.

## Render Markdown

```python
pdf = ironpress.markdown_to_pdf("# Release notes\n\nEverything shipped.")
```

## Configure and reuse a converter

```python
converter = ironpress.HtmlConverter()
converter.page_size("Letter")
converter.margin_sides(36, 48, 36, 48)
converter.header("Quarterly report")
converter.footer("Page {page} of {pages}")
# Use header_html/footer_html for sanitized images, tables, and styled markup.
converter.header_html("<strong>Quarterly report</strong>")

pdf = converter.convert("<h1>Results</h1>")
markdown_pdf = converter.convert_markdown("# Results")
```

Python configuration methods mutate the converter. Use `convert_to_file` or
`convert_markdown_to_file` when direct output is more convenient than bytes.
`accessible(True)`, with `document_title` and `document_language`, produces a
Tagged PDF conforming to PDF/UA-1; an invalid language tag or a blank title
raises `ValueError`.

## Local resources and fonts

Local URLs are denied until a canonical boundary is configured:

```python
from pathlib import Path

converter = ironpress.HtmlConverter()
converter.base_path("templates")
converter.resource_root(".")
converter.add_font("Inter", Path("assets/Inter.ttf").read_bytes())
```

Optional fallback packs enter through `add_font_pack`. Valid names are
`cjk-jp`, `cjk-kr`, `cjk-sc`, `cjk-tc`, and `emoji`. Rendering never downloads
a font pack.

## Limits and errors

- Invalid configuration and conversion failures raise `ValueError`.
- HTML sanitization is enabled by default.
- The binding has no async or streaming conversion API.
- Remote HTTP document resources are not available.
- JavaScript and browser DOM execution are not supported.

See the [binding capability matrix](../README.md) for the contract shared with
Rust, Ruby, browser WebAssembly, and Node.js.
