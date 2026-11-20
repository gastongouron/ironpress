# Ironpress for .NET

Ironpress renders HTML, CSS, and Markdown to PDF bytes inside a .NET process.
The NuGet package contains the managed facade and its native Ironpress library;
consumers do not need Rust, Chrome, or a subprocess.

## Install

```bash
dotnet add package Ironpress
```

The initial package targets .NET 8 or newer and includes native assets for:

- Linux x64 and ARM64 with glibc 2.28 or newer
- macOS x64 and ARM64
- Windows x64

## Convert HTML

```csharp
using Ironpress;

using var converter = new HtmlConverter()
    .SetPageSize(PageSize.Letter)
    .SetMargin(36)
    .SetFooter("Page {page} of {pages}");

byte[] pdf = converter.ConvertHtml("<h1>Hello from .NET</h1>");
File.WriteAllBytes("output.pdf", pdf);
```

`ConvertMarkdown` accepts Markdown and returns the same owned `byte[]` result.
Use `SetHeaderHtml` or `SetFooterHtml` for sanitized images, tables, and styled
markup in the page margins.

## Configure rendering

The managed facade covers the portable C ABI contract:

- named or custom page dimensions and per-side margins
- compression, JPEG quality, image resizing, and raster resolutions
- sanitization, headers, and footers
- accessible output (Tagged PDF, PDF/UA-1) with `SetAccessibility`,
  `SetDocumentTitle`, and `SetDocumentLanguage`
- custom TrueType font bytes
- optional Japanese, Korean, Simplified Chinese, Traditional Chinese, and emoji packs

Every mutating method returns the same converter for fluent configuration.
`HtmlConverter` owns a native resource and implements `IDisposable`; use a
`using` declaration or call `Dispose` deterministically. A converter may move
between threads while idle, but it must not be used or disposed concurrently.

## Errors

Native failures throw `IronpressException`. Its `Kind` property retains the
stable parser, layout, render, font, security, or argument category. Ordinary
.NET contract violations, such as invalid custom page dimensions or use after
disposal, use the standard argument and disposal exception types.

## Fonts and security

The package never downloads fonts. Load an optional pack from an Ironpress
release and pass its bytes explicitly:

```csharp
using var converter = new HtmlConverter()
    .AddFontPack(
        FontPackKind.CjkJapanese,
        File.ReadAllBytes("ironpress-font-cjk-jp.ttf"));
```

The first .NET contract accepts document and font bytes only. It does not expose
local paths, direct file output, streaming, asynchronous conversion, or remote
resource access. HTML sanitization remains enabled by default.

The managed assembly verifies ABI generation 1 before allocating a converter.
See the shared [binding matrix](../README.md) and the native
[ABI contract](../c/ABI.md) for the complete ownership model.
