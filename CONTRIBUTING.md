# Contributing to ironpress

Thanks for your interest in contributing. Bug reports, focused fixes, tests,
documentation, and support for language bindings are all welcome.

Participation in this project is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).
Report suspected vulnerabilities through the private process in
[SECURITY.md](SECURITY.md), not through a public issue.

## Before starting

Search the existing issues and pull requests before opening a new one. Use the
bug or feature form so the report includes the runtime, version, platform, and a
minimal example.

For a substantial feature, public API change, or new CSS behavior, open an issue
before writing the implementation. This gives maintainers and contributors a
place to confirm the requirement, applicable specification, and scope.

## Development workflow

1. Fork the repository and branch from `main`.
2. Keep each change focused on one problem.
3. Establish the expected behavior from an independent requirement,
   specification, verified oracle, or reproduced regression.
4. Add or update tests with the implementation.
5. Run the checks that apply to the changed area.
6. Open a pull request against `main` and complete its verification notes.

## Core checks

The minimum supported Rust version is 1.88. Run the core checks from the
repository root:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test --verbose
cargo test --verbose --features remote
```

CI also verifies the WebAssembly target with Rust 1.88:

```bash
cargo check --target wasm32-unknown-unknown --no-default-features --features wasm
```

## Area-specific checks

Run the checks closest to the files you changed. The GitHub workflows remain the
source of truth for complete platform and release-package matrices.

| Area | Local checks |
|---|---|
| C ABI | `cargo test -p ironpress-ffi`, `bindings/c/generate-header.sh --check`, `bindings/c/tests/run.sh` |
| Conan and vcpkg | Build both linkage modes with the commands in `packaging/README.md` |
| .NET | `dotnet build bindings/dotnet/src/Ironpress/Ironpress.csproj --configuration Release` |
| Java | `mvn -B -f bindings/java/pom.xml clean verify` after preparing the native assets described by the Java workflow |
| Python | Install the built wheel, then run `python -m unittest discover -s bindings/python/tests -v` |
| Ruby | Run `bundle exec rake` from `bindings/ruby` |
| Browser and Node.js WASM | `scripts/build-wasm-package.sh`, then `node scripts/test-node-package.mjs` |
| Website | Run `npm ci --ignore-scripts`, `npm test`, and `npm run check` from `scripts` |
| Accessible output | Convert `tests/pdfua/report.html` and `tests/pdfua/effects.html` with `--accessible` and validate both with veraPDF (`--flavour ua1`), as the `pdfua` CI job does; output without `--accessible` must stay byte-identical |

See the workflows in [`.github/workflows`](.github/workflows) for required tool
versions, native targets, package layout checks, and release verification.

## Rendering and parity changes

Ironpress keeps raw visual evidence for every parity fixture. A rendering change
must not update an expectation only to match the implementation.

- Add the smallest fixture that establishes the behavior.
- Link the applicable specification or independently verified regression.
- Run `scripts/parity.sh` when rendered output can change.
- Keep `tests/parity/REPORT.md` and `tests/parity/report.json` current.
- Generate a new oracle PDF only with the pinned Chromium launcher.
- Run `scripts/parity-gen-refs.sh --check` after any retained oracle change.

Do not translate, crop, resize, filter, or otherwise align comparison rasters.
Differences must remain visible at their original page coordinates.

## Guidelines

- Production code must not panic.
- Add tests for new functionality and reproduced bugs.
- Keep commits and pull requests focused on one logical change.
- Document public API and user-visible behavior changes.
- Update `CHANGELOG.md` when a change affects released users.
- Follow the conventions in the surrounding code and project instructions.

## Code comments

The codebase is intentionally low on comments. Follow these conventions:

- **`///` doc comments** on public and most private structs, enums, fields, and
  functions. Explain *why* when code encodes a non-obvious specification rule or
  design choice. A bare *what* is not enough for layout or CSS logic.
- **`//!` module-level docs** only when a file has a meaningful architectural
  role worth describing. Not on every file.
- **`//` inline comments** sparingly, only for non-trivial algorithmic steps.
- **No block comments** (`/* */`).
- **No `FIXME` or `HACK` markers.** `TODO:` is acceptable on `#[ignore]` tests.
- Let well-named identifiers speak for themselves. Do not narrate what the code
  does. Document why it does it that way.

## Reporting Issues

Use the issue forms with a clear description and a minimal reproduction. Attach
the generated PDF, reference output, visual diff, error, or backtrace when it
helps explain the problem.
