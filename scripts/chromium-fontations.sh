#!/usr/bin/env bash
# Launch the authenticated Chromium oracle in Fontations-only mode.

set -euo pipefail

readonly FONTATIONS_FEATURES="FontationsFontBackend,FontationsLinuxSystemFonts"
if [ "$#" -eq 1 ] && [ "$1" = "--print-features" ]; then
  echo "$FONTATIONS_FEATURES"
  exit 0
fi

chromium="${IRONPRESS_CHROMIUM_EXECUTABLE:-}"
if [ -z "$chromium" ] || [ ! -x "$chromium" ]; then
  echo "chromium-fontations: IRONPRESS_CHROMIUM_EXECUTABLE must name an executable" >&2
  exit 1
fi
if [ "$chromium" = "$0" ]; then
  echo "chromium-fontations: refusing a recursive Chromium launcher" >&2
  exit 1
fi
for argument in "$@"; do
  case "$argument" in
    --disable-features=*Fontations*)
      echo "chromium-fontations: Fontations may not be disabled" >&2
      exit 1
      ;;
  esac
done

# Custom Fontconfig files make Chromium instantiate system faces through its
# legacy metric path even when the Fontations features are enabled. The parity
# fonts are installed in the default user font directory and authenticated by
# refs.lock, so the oracle launcher must use Chromium's default font discovery.
unset FONTCONFIG_FILE FONTCONFIG_PATH
export FC_FONTATIONS=1
exec "$chromium" "$@" \
  --enable-features="$FONTATIONS_FEATURES"
