# Ironpress

Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.

## Installation

```ruby
gem install ironpress
```

Or in your Gemfile:

```ruby
gem "ironpress"
```

## Usage

```ruby
require "ironpress"

# HTML to PDF
pdf = Ironpress.html_to_pdf("<h1>Hello World</h1><p>Generated with Ironpress.</p>")
File.binwrite("output.pdf", pdf)

# Markdown to PDF
pdf = Ironpress.markdown_to_pdf("# Hello\n\nGenerated from **Markdown**.")
File.binwrite("output.pdf", pdf)
```

## Performance

- **10-100x faster** than browser-based solutions (Puppeteer, wkhtmltopdf)
- **~5MB** native extension, no runtime dependencies
- Instant startup, no browser process
