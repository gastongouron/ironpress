# ironpress

Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.

## Installation

```bash
pip install ironpress
```

## Quick Start

```python
import ironpress

# Simple conversion
pdf = ironpress.html_to_pdf("<h1>Hello World</h1><p>Generated with ironpress.</p>")
with open("output.pdf", "wb") as f:
    f.write(pdf)

# Markdown
pdf = ironpress.markdown_to_pdf("# Hello\n\nGenerated from **Markdown**.")

# Advanced options
converter = ironpress.HtmlConverter()
converter.page_size("Letter")
converter.landscape(True)
converter.margin(36.0)  # 0.5 inch margins
converter.header("My Document")
converter.footer("Page {page} of {pages}")
pdf = converter.convert("<h1>Landscape PDF</h1>")

# Custom fonts
ttf_data = open('Inter.ttf', 'rb').read()
converter = ironpress.HtmlConverter()
converter.add_font('Inter', ttf_data)
pdf = converter.convert('<p style="font-family: Inter">Shaped with HarfBuzz</p>')
```

## API

### `html_to_pdf(html: str) -> bytes`
Convert an HTML string to PDF bytes.

### `markdown_to_pdf(markdown: str) -> bytes`
Convert a Markdown string to PDF bytes.

### `HtmlConverter`
Configurable converter with options:
- `page_size(name)` — `"A4"`, `"Letter"`, or `"Legal"`
- `landscape(enabled)` — landscape orientation
- `margin(points)` — uniform margin in points (72 points = 1 inch)
- `margin_sides(top, right, bottom, left)` — margin with individual values for each side (72 points = 1 inch)
- `header(header)` — header text
- `footer(footer)` — footer text
- `add_font(font_name, ttf_data)` — custom font support
- `base_path(path)` — base path for custom CSS file imports
- `convert(html)` → PDF bytes
- `convert_markdown(markdown)` → PDF bytes

## Performance

- **10-100x faster** than browser-based solutions (Puppeteer, Playwright)
- **~5MB** binary, no runtime dependencies
- Instant startup, no browser process
