#!/usr/bin/env bash
# Bump version across the Rust crate, Python wheel, Ruby gem, and binding sub-crates.
# Usage: ./scripts/bump-version.sh <new-version>
#
# Updates:
#   Cargo.toml                          (ironpress crate)
#   bindings/python/pyproject.toml      (PyPI wheel)
#   bindings/python/Cargo.toml          (ironpress-python internal crate)
#   bindings/ruby/ironpress.gemspec     (RubyGems)
#   bindings/ruby/Cargo.toml            (ironpress-ruby internal crate)
#
# Run `cargo check` afterwards to refresh Cargo.lock.
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <new-version>" >&2
    echo "Example: $0 1.4.3" >&2
    exit 1
fi

NEW="$1"

# Validate semver-ish: N.N.N or N.N.N-pre
if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    echo "Error: '$NEW' is not a valid version (expected X.Y.Z or X.Y.Z-pre)" >&2
    exit 1
fi

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_DIR"

bump_toml_version() {
    local file="$1"
    # Replace the first `version = "..."` line (the [package] version, which comes first).
    perl -i -pe 'BEGIN{$n=0} if (!$n && /^version\s*=\s*"[^"]+"/) { s/"[^"]+"/"'"$NEW"'"/; $n=1 }' "$file"
}

bump_pyproject() {
    local file="$1"
    # [project] table owns `version = "..."`; match the first one (top of file).
    perl -i -pe 'BEGIN{$n=0} if (!$n && /^version\s*=\s*"[^"]+"/) { s/"[^"]+"/"'"$NEW"'"/; $n=1 }' "$file"
}

bump_gemspec() {
    local file="$1"
    # spec.version = "..."
    perl -i -pe 's/(spec\.version\s*=\s*)"[^"]+"/$1"'"$NEW"'"/' "$file"
}

echo "Bumping to $NEW"

bump_toml_version  "Cargo.toml"                        && echo "  Cargo.toml"
bump_pyproject     "bindings/python/pyproject.toml"    && echo "  bindings/python/pyproject.toml"
bump_toml_version  "bindings/python/Cargo.toml"        && echo "  bindings/python/Cargo.toml"
bump_gemspec       "bindings/ruby/ironpress.gemspec"   && echo "  bindings/ruby/ironpress.gemspec"
bump_toml_version  "bindings/ruby/Cargo.toml"          && echo "  bindings/ruby/Cargo.toml"

echo ""
echo "Done. Run 'cargo check' to refresh Cargo.lock, then review the diff."
