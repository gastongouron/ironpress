# Ironpress for C++

Ironpress provides a header-only C++17 wrapper over its stable C ABI. It keeps
native converter and PDF allocations under move-only RAII ownership, while the
Rust renderer remains the only implementation of document behavior.

## Install

Download the archive for your platform from an Ironpress GitHub release and
point `CMAKE_PREFIX_PATH` at its root. The default target uses the shared native
library:

```cmake
find_package(Ironpress CONFIG REQUIRED)
target_link_libraries(your_target PRIVATE Ironpress::CXX)
```

Use `Ironpress::CXXStatic` for static linkage. Both targets provide the include
path, require C++17, and carry the native link contract transitively.

The repository also maintains source recipes for Conan 2 and vcpkg. They build
and test `Ironpress::CXX` from the tagged source on the full native matrix before
submission to the public registries. See the [native package manager
guide](../../packaging/README.md) to validate or consume the in-repository
recipes. GitHub release archives remain the public installation path until the
external submissions are accepted.

The archive contains both `ironpress.hpp` and `ironpress.h`. Keep these headers
paired with the library from the same archive. The wrapper checks the linked ABI
generation before allocating or converting.

## Convert HTML

```cpp
#include "ironpress.hpp"

#include <fstream>

int main() {
    ironpress::Converter converter;
    converter.set_page_size(ironpress::PageSize::letter)
        .set_margins(ironpress::PageMargins::uniform(36.0F))
        .set_footer("Page {page} / {pages}");

    const auto pdf = converter.convert_html("<h1>Hello from C++</h1>");
    std::ofstream output("output.pdf", std::ios::binary);
    output.write(reinterpret_cast<const char*>(pdf.data()),
                 static_cast<std::streamsize>(pdf.size()));
}
```

Use `set_header_html` or `set_footer_html` for sanitized images, tables, and
styled markup in page margins. The plain setters never infer HTML.
`set_accessibility(true)`, with `set_document_title` and
`set_document_language`, produces a Tagged PDF conforming to PDF/UA-1.

`Converter` and `Pdf` cannot be copied. Moving either object transfers its one
native owner, and destruction releases that owner through the matching C ABI
function. A moved-from object is empty and may be destroyed or assigned again.

## Errors

Fallible methods throw `ironpress::Error`. Its `status()` method exposes a
stable `ironpress::Status` category, while `native_status()` preserves an
unknown status from a newer library. The diagnostic in `what()` is copied
before the native error owner is released.

Validated value constructors use `std::invalid_argument`, matching normal C++
argument handling before any native call occurs.

No C++ exception crosses the native boundary. Rust panics are caught by the C
ABI, returned as `Status::internal`, and only then translated into a C++
exception. Destructors never throw.

## Fonts and binary input

`ironpress::BytesView` borrows bytes without copying them. Its source must stay
alive for the complete call:

```cpp
std::vector<std::uint8_t> font = read_font();
converter.add_font("Inter", ironpress::BytesView(font));
```

The same API accepts the five optional CJK and emoji packs through
`add_font_pack`.

## Threads and capabilities

A converter may move between threads while idle. Do not configure, convert, or
destroy the same owner concurrently. PDF byte views remain valid until their
`Pdf` owner is destroyed or moved.

The wrapper exposes the same portable contract as the C ABI: HTML and Markdown
PDF bytes, reusable converters, page geometry, quality controls, sanitization,
headers, footers, custom fonts, and optional font packs. It does not read local
paths or enable remote resources.

## Build from source

```bash
cargo build --release -p ironpress-ffi
IRONPRESS_PROFILE=release bindings/cpp/tests/run.sh
```

CI extracts the exact release archive, then compiles and runs CMake consumers
against static and shared libraries with GCC, Clang, and MSVC.
