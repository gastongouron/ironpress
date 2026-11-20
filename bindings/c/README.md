# Ironpress for C

Ironpress exposes its portable HTML and Markdown conversion contract through a
stable C ABI. The same native boundary supports the .NET, Java, and C++ bindings
without exposing Rust layouts or requiring a Rust toolchain at runtime.

## Install

Each Ironpress GitHub release provides one archive for every initial native
platform:

| Archive suffix | Platform |
|---|---|
| `linux-x86_64` | Linux x86_64, glibc 2.28+ |
| `linux-aarch64` | Linux ARM64, glibc 2.28+ |
| `macos-x86_64` | macOS Intel |
| `macos-aarch64` | macOS Apple Silicon |
| `windows-x86_64` | Windows x86_64, MSVC |

Every archive contains the C header, the header-only C++17 wrapper, a shared
library, a static library, both native guides, the ABI contract, and the
license. All archives include relocatable CMake metadata; Unix archives also
include pkg-config metadata. `SHA256SUMS` in the release verifies the archives.

Use the shared C target by default:

```cmake
find_package(Ironpress CONFIG REQUIRED)
target_link_libraries(your_target PRIVATE Ironpress::C)
```

Use `Ironpress::CStatic` for static linkage. Unix C consumers may instead set
`PKG_CONFIG_PATH` to the archive's `lib/pkgconfig` directory and resolve
`ironpress` through pkg-config. The separate `ironpress-static` module selects
the archive and carries the extra system libraries required by static linkage.

The repository also maintains source recipes for Conan 2 and vcpkg. They build
and test `Ironpress::C` from the tagged source on the full native matrix before
submission to the public registries. See the [native package manager
guide](../../packaging/README.md) to validate or consume the in-repository
recipes. GitHub release archives remain the public installation path until the
external submissions are accepted.

## Convert HTML

```c
#include "ironpress.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    IronpressBuffer *pdf = NULL;
    IronpressError *error = NULL;
    const char *html = "<h1>Hello from C</h1>";
    IronpressBytes input = {
        .data = (const uint8_t *)html,
        .len = strlen(html),
    };

    IronpressStatus status = ironpress_html_to_pdf(input, &pdf, &error);
    if (status != IRONPRESS_STATUS_OK) {
        fwrite(ironpress_error_message_data(error), 1,
               ironpress_error_message_len(error), stderr);
        ironpress_error_free(&error);
        return 1;
    }

    FILE *output = fopen("output.pdf", "wb");
    if (output == NULL) {
        ironpress_buffer_free(&pdf);
        return 1;
    }
    fwrite(ironpress_buffer_data(pdf), 1, ironpress_buffer_len(pdf), output);
    fclose(output);
    ironpress_buffer_free(&pdf);
    return 0;
}
```

For repeated conversions, allocate one `IronpressConverter`, configure it, and
reuse it. The generated header documents every symbol. See [ABI.md](ABI.md) for
the compatibility, ownership, error, and threading contracts.

## Capabilities

Generation 1 covers HTML and Markdown PDF bytes, named and custom page sizes,
per-side margins, compression and raster controls, sanitization, headers,
footers, custom TrueType fonts, and optional CJK or emoji packs.
Rich page margins use `ironpress_converter_set_header_html` and
`ironpress_converter_set_footer_html`; the existing setters remain plain text.
`ironpress_converter_set_accessible`, with
`ironpress_converter_set_document_title` and
`ironpress_converter_set_document_language`, produces a Tagged PDF conforming
to PDF/UA-1.

Local paths, direct file output, streaming, asynchronous conversion, and remote
resources are intentionally absent. Applications provide document and font
bytes explicitly, and the binding never enables network access.

## Build from source

```bash
cargo build --release -p ironpress-ffi
bindings/c/tests/run.sh
```

The build produces static and shared libraries in `target/release`. Regenerate
the committed header with the pinned `cbindgen` release:

```bash
cargo install cbindgen --version 0.29.4 --locked
bindings/c/generate-header.sh
```

CI rejects a header that does not match the Rust declarations. It links the C
lifecycle test against both library forms. Linux runs the debug library under
Valgrind and exercises the optimized release library separately. Keeping memory
diagnostics on the unoptimized build avoids compiler-generated padding noise
without weakening the release ABI test.
