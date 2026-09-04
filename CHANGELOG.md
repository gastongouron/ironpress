# Changelog

## Unreleased

### Added

- Conan 2 and vcpkg source recipes package the existing C and C++ bindings and
  verify static and shared consumers across the native platform matrix.
- SVG `<text>` honors `letter-spacing` from the presentation attribute or an
  inline style with any CSS `<length>` (`em` resolves against the text's font
  size). Tracking is applied per typographic character unit: zero-width
  formatting characters receive none, optional ligatures are suppressed while
  tracking is active, and the tracked advance positions `text-anchor`.

### Changed

- Font setup reuses bounded process-lifetime caches: `add_font`/`@font-face`
  faces and system-font resolution are memoized in capped LRU tables, and every
  parsed font owns its shaping face, so a warm process stops re-parsing and
  re-resolving the same fonts on each `convert()` — with byte-identical output.
- Auto table layout memoizes each cell's intrinsic widths within a layout,
  avoiding repeated nested-table measurement while preserving byte-identical
  output. The retained Criterion benchmark covers nesting depths 1 through 8.

### Fixed

- SVG `<text>` with a CSS font-family list (`"MyFace, Helvetica"`) resolves
  registered custom faces, including quoted names that contain commas, and
  every font the SVG renderer binds is also subset and embedded; `<text>`
  inside CSS background-image SVGs uses the registered custom fonts instead of
  always falling back to standard fonts.
- A `display: inline-block` inside a table cell (form fill-in underlines,
  checkbox squares) flows inline with the cell's sibling text regardless of
  the element's default HTML display role; lone and relatively positioned
  inline boxes are painted by the same flow.

## [1.6.0] — 2026-08-26

### Added

- Explicit rich-header and rich-footer APIs render sanitized HTML fragments,
  including images and tables, through CSS running elements on every page.

## [1.5.5] — 2026-08-25

### Added

- A header-only C++17 binding provides move-only RAII owners, typed errors, and
  the complete portable converter contract over the stable C ABI.
- Native release archives provide relocatable CMake targets for C and C++, plus
  pkg-config metadata for Unix C consumers.

### CI

- C++ consumers render through GCC, Clang, and MSVC, then repeat the smoke test
  from the exact native archive prepared for release.
- CMake consumers link static and shared archives on every native platform;
  Unix jobs also verify pkg-config discovery and transitive static link flags.

### Documentation

- The binding matrix, native guides, README, and public site include the C++
  install, ownership, error, font, and runtime contracts.

## [1.5.4] — 2026-08-23

### Added

- A versioned C ABI exposes the portable converter contract through opaque
  handles and ships native libraries for Linux, macOS, and Windows.
- A .NET 8+ `HtmlConverter` package uses the stable C ABI through `SafeHandle`
  ownership and ships native assets for five initial runtime identifiers.
- A Java 17+ `HtmlConverter` package uses the stable C ABI through JNA,
  `AutoCloseable` ownership, and typed renderer exceptions.
- The existing npm package now exposes `ironpress/node`, an ESM entry point that
  loads its packaged WebAssembly binary without consumer-side file handling.

### Fixed

- Ruby native gems load their extension from the directory for the active Ruby
  API version on every supported platform.

### CI

- NuGet artifacts are installed and rendered on Linux, macOS, and Windows
  before OIDC trusted publishing can run.
- Reproducible Maven artifacts are installed from their embedded POM and
  rendered on all five supported native platforms before publication.
- The packed npm artifact is installed, type-checked, and rendered through every
  supported Node.js LTS line before that exact artifact can be published.
- Static site checks parse HTML through a standards-compliant tree instead of
  filtering source markup with regular expressions.
- Ruby native-gem builds use the crate-aware extension task and a writable Ruby
  environment.
- Crates.io availability checks identify the release workflow instead of timing
  out on an anonymous request.
- Font-pack assets target the current repository explicitly when attached to a
  GitHub release.

### Documentation

- A canonical Get Started hub now covers Rust, the CLI, Python, Ruby, browser
  JavaScript, and Node.js with the same task-oriented structure.
- The website, root README, binding matrix, and package READMEs route users to
  the guide for their runtime.

## [1.5.3] — 2026-08-20

### Added

- Rust and WebAssembly converters can install explicit fallback font packs.
- Python, Ruby, and WebAssembly expose the same portable converter controls for
  page geometry, quality, sanitization, headers, footers, and fonts.
- GitHub releases publish reproducible Japanese, Korean, Simplified Chinese,
  Traditional Chinese, and monochrome emoji font artifacts.
- CJK fallback follows inherited HTML `lang` values, including nested language
  changes, so regional glyph forms are selected intentionally.

### Changed

- Browser/WASM builds no longer embed the incomplete CJK subset or the full
  emoji face. Applications that need this coverage must load the matching pack.
- Native builds may still use compatible system fonts when no pack is installed.
- Python uses one CPython 3.8+ ABI3 wheel per platform instead of binding wheels
  to the build interpreter.

### Fixed

- Ruby releases build installable native extensions instead of relabeling the
  same source gem as platform-specific artifacts.

### CI

- Python and Ruby packages are installed and exercised before publication.
- Parity runtime downloads are pinned, checksum-verified, cached, and bounded by
  explicit network, installation, step, and job timeouts.
- `wasm-pack` is pinned and the generated npm version is checked against the
  Rust crate version.

### Documentation

- The repository and public site document the shared binding contract and the
  capabilities that remain native-only or Rust-only.

## [1.5.2] — 2026-08-17

### Changed

- HTML parsing now uses `html5ever` and `markup5ever_rcdom` 0.39.
- The declared MSRV is now Rust 1.88, matching the stable language features
  already required by Ironpress. CI now verifies it directly.

### Fixed

- CSS strings preserve non-ASCII text, and CSS escapes are decoded in
  `@page` margin content (#203, #204).
- Page margin boxes fall back past unavailable font families (#205).
- `calc()` resolves custom properties instead of dropping declarations that
  contain `var()` (#206).
- Named pages are selected from the document root (#207).
- Absolutely positioned boxes retain `top` and `left` offsets when they
  contain block-level children (#208).

### Security

- Tokio's supported version range now excludes releases affected by
  RUSTSEC-2023-0001.

### Compatibility

- WebAssembly keeps `getrandom` 0.3 until `lightningcss` moves off that major,
  preserving its required `wasm_js` feature wiring.

### CI

- Crates.io publishing checks the remote registry before deciding that a
  version already exists.

### Documentation

- The README now links to the canonical Chromium parity report.

## [1.5.1] — 2026-08-15

### Fixed

- System font families now resolve case-insensitively through `fontdb`,
  without relying on `fc-match` as a fallback (#189).
- Nested flex items now use intrinsic content sizing, flex rows grow around
  their content, and empty auto-height flex boxes retain margins (#190–#192).
- Inline elements retain horizontal padding and margins. NBSP, EN SPACE, and
  EM SPACE keep their advances under normal whitespace handling (#196, #197).

### Security

- Document resources now use explicit per-conversion authorization. Local
  files are denied until `base_path` or `resource_root` grants a canonical
  directory; traversal and symlink escapes are rejected.
- Remote fetching now checks schemes, host allow/deny lists, non-public address
  classes, every redirect, pinned DNS results, and response-size limits.
- HTML sanitization no longer controls resource authorization.
  `.sanitize(false)` does not grant local or remote access; projects that
  relied on working-directory file access must configure `.base_path(...)`.

### Performance

- Resource loads, including failures, are cached for one conversion. Distinct
  remote image URLs are preloaded with at most eight concurrent requests (#195).

### Tests

- `float: right` and JPEG background regressions now run against rendered PDF
  output instead of remaining ignored.

### Documentation

- The README links to the complete resource-security threat model and server
  deployment guidance in the wiki.

## [1.5.0] — 2026-08-09

### Added

- Python now exposes the complete `HtmlConverter` builder, custom fonts,
  file output, and Markdown conversion.
- Ruby now provides `Ironpress::HtmlConverter` with the same builder controls
  as the Rust API.

### Fixed

- Inline and atomic inline siblings keep their source order when a block
  splits the surrounding formatting context.

### Security

- Python bindings use PyO3 0.29.
- The parity workflow now has explicit read-only repository permissions.

### CI

- Parity and playground reports use the same pinned Poppler and font runtime.

### Documentation

- Benchmarks now include Apple M2 results, headers and footers, and CJK text
  emphasis.

## [1.4.4] — 2026-07-29

### Chromium visual parity

Ironpress now achieves **100% verified visual parity** with Chromium across a
1,662-fixture adversarial corpus (1,642 PASS / 0 FAIL / 20 reference-disputed).
Every fixture comparison uses the same pinned `pdftoppm` invocation at 300 DPI —
no translation, registration, jitter, or raster replacement.

### Parity test harness

- Same-rasterizer parity gate: `scripts/parity.sh` renders every fixture
  in-process, rasterizes candidate and oracle PDFs symmetrically, and reports
  pass/fail with complete RGBA evidence.
- `refs.lock` authenticates each fixture, oracle PDF, renderer, fonts, and
  provenance.
- `baseline.json` tracks regression health separately from test history.
- HTML parity report with full visual diffs for every fixture.
- CI gate (`.github/workflows/parity.yml`) runs the same browser-free check.

### Layout & rendering fixes

- **Paged media**: `@page` backgrounds cascade through named, `:first`, `:left`,
  `:right`, and `:blank` selectors; sheet decoration modeled semantically.
- **Multicol**: block flow constraints, nested clip paths, rule positioning,
  and layout split by semantic responsibility.
- **Grid**: block-size constrained before track alignment; multicol reference
  geometry corrected.
- **Borders**: collapsed table borders resolved as a shared grid; opaque square
  painting unified; rounded background coverage aligned with Chromium; vector
  serialization matches Chromium; bevel geometry matched.
- **Filters**: inherited filter layer paint space; premultiplied source
  rendering; raster placement and sampling; linear-light surface parity;
  descendant clipping to overflow bounds; filter surface geometry fix.
- **Inline layout**: mixed advances unified; atomic inline origins preserved;
  generated content traversal unified; layout probes replaced with capabilities.
- **Text**: origin-aware text decorations; text combine expansion handling.
- **Tables**: cell paint phase ordering; expanded table height honored in flex
  alignment.
- **Images**: source pixels preserved for `object-fit: cover`; source images
  reused across page fragments; fragmented JPEG background ownership; off-fragment
  resources culled; certificate image rendering optimized.
- **Backgrounds**: nested fills routed through box background geometry; rounded
  background raster phase fixed; full-box radial masks rendered as native shadings.
- **Graphical effects**: continued across page boxes; transformed descendants
  composited in filter sources; descendant clips propagated into filter sources.
- **Transforms**: Fontations transform oracle restored; fractional transform
  parity; CSS print scale reconstructed exactly.
- **Raster**: bounds quantized from exact DPI; x-height quantization matched
  with Fontations.
- **List markers**: built-in markers rendered as vector shapes.
- **Opacity**: applied to absolute-positioned elements.
- **Box-shadow**: parity pass corrections.
- **Percentage widths**: parity pass corrections.
- **Heading font-sizes**: use `em` units (Chrome UA parity).
- **Body padding**: folded into page margin.

### Performance

- README benchmark claims replaced with reproducible Criterion medians.
- Benchmark profile: `opt-level=3`, fat LTO, one codegen unit.

### Builder API

- `RasterQuality` struct: controls background, filter, and image DPI in one
  policy. CLI exposes `--image-dpi`, `--filter-dpi`, `--background-raster-dpi`.

### CI

- Parity rasterizer pinned in CI.
- Codecov made informational for parity PRs.
- Fork PR visual ref regeneration avoided.
- Synthetic font resource test made host-independent.
