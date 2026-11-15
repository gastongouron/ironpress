# Language bindings

Ironpress exposes one rendering engine through Rust, C, C++, .NET, Java, Python,
Ruby, browser WebAssembly, and Node.js WebAssembly. Each binding keeps the
conventions of its runtime while sharing the portable converter contract below.

## Capability matrix

| Capability | Rust | C ABI | C++ | .NET | Java | Python | Ruby | Browser WASM | Node.js WASM |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| HTML and Markdown to PDF bytes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Reusable configured converter | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Page size and margins | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| PDF and raster quality controls | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Sanitization | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Plain-text and HTML headers/footers | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Accessible Tagged PDF (PDF/UA-1) with title and language | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Custom TTF fonts | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Optional CJK and emoji packs | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Local resource boundary | Yes | No | No | No | No | Yes | Yes | No | No |
| Direct file output | Yes | No | No | No | No | Yes | Yes | No | No |
| Streaming writer and async API | Yes | No | No | No | No | No | No | No | No |
| Remote HTTP resources | Opt-in | No | No | No | No | No | No | No | No |

WebAssembly receives font and document bytes from the host application. It does
not read local paths. Remote resource loading remains available only through the
Rust crate's opt-in `remote` feature so language packages do not silently gain
network access.

## Published artifacts

| Runtime | Package | Supported artifact contract |
|---|---|---|
| Rust | [`ironpress`](https://crates.io/crates/ironpress) | Source crate, Rust 1.88+ |
| C | [GitHub Releases](https://github.com/gastongouron/ironpress/releases) | CMake metadata, Unix pkg-config files, header, and native libraries |
| C++ | [GitHub Releases](https://github.com/gastongouron/ironpress/releases) | CMake targets and C++17 RAII headers with the native libraries |
| .NET | [`Ironpress`](https://www.nuget.org/packages/Ironpress) | .NET 8+ managed assembly with Linux, macOS, and Windows native assets |
| Java | [`io.github.gastongouron:ironpress`](https://central.sonatype.com/artifact/io.github.gastongouron/ironpress) | Java 17+ JAR with Linux, macOS, and Windows native assets |
| Python | [`ironpress`](https://pypi.org/project/ironpress/) | CPython 3.8+ ABI3 wheels for Linux, macOS, and Windows |
| Ruby | [`ironpress`](https://rubygems.org/gems/ironpress) | Ruby 3.0+ source gem and native gems for Linux, macOS, and Windows |
| Browser WebAssembly | [`ironpress`](https://www.npmjs.com/package/ironpress) | Browser ESM entry and WebAssembly binary |
| Node.js WebAssembly | [`ironpress/node`](https://www.npmjs.com/package/ironpress) | Node.js 22/24 ESM entry using the same WebAssembly binary |

All published packages use the same Ironpress version. CI builds and exercises
the C, C++, .NET, Java, Python, and Ruby artifacts before a release can publish them.
The NuGet and Maven packages are installed and rendered on every advertised
platform without Rust.
The generated npm tarball is installed, type-checked, and rendered through
Node.js 22 and 24; the browser entry remains a separate required check.

Start with the guide for your runtime:

- [Rust](https://gastongouron.github.io/ironpress/get-started/rust/)
- [CLI](https://gastongouron.github.io/ironpress/get-started/cli/)
- [C](c/README.md)
- [C++](https://gastongouron.github.io/ironpress/get-started/cpp/)
- [.NET](https://gastongouron.github.io/ironpress/get-started/dotnet/)
- [Java](https://gastongouron.github.io/ironpress/get-started/java/)
- [Python](https://gastongouron.github.io/ironpress/get-started/python/)
- [Ruby](https://gastongouron.github.io/ironpress/get-started/ruby/)
- [Browser JavaScript](https://gastongouron.github.io/ironpress/get-started/browser/)
- [Node.js](https://gastongouron.github.io/ironpress/get-started/node/)
