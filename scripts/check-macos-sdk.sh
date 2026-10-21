#!/usr/bin/env bash
set -euo pipefail

# Read-only diagnostics for the active toolchain. This never installs tools,
# runs xcode-select -s or treats a missing test runner as a pass.
# CLT can provide the macOS SDK/compiler without providing xcodebuild/XCTest.
if [[ "$#" != 1 ]]; then
  printf 'Uso: bash scripts/check-macos-sdk.sh sdk|xctest\n' >&2
  exit 64
fi

# Bounded, overridable list of places an Xcode may already live. Nothing here
# descends into arbitrary directories or follows a search of the whole disk.
orbit_developer_candidates() {
  local roots="${ORBIT_XCODE_SEARCH_PATHS-/Applications:$HOME/Applications}"
  local root bundle
  # `|| [[ -n "$root" ]]` keeps the last entry when the list has no trailing
  # newline; without it a single-root list would be read and then discarded.
  while IFS= read -r root || [[ -n "$root" ]]; do
    [[ -n "$root" && -d "$root" ]] || continue
    for bundle in "$root"/Xcode*.app; do
      [[ -x "$bundle/Contents/Developer/usr/bin/xctest" ]] || continue
      printf '%s\n' "$bundle/Contents/Developer"
    done
  done < <(printf '%s' "$roots" | tr ':' '\n')
}

case "$1" in
  sdk)
    xcode-select --print-path
    swift --version
    orbit_sdk_path="$(xcrun --sdk macosx --show-sdk-path)"
    if [[ "$orbit_sdk_path" != /* || ! -d "$orbit_sdk_path" ]]; then
      printf '::error::xcrun no identifica un directorio SDK de macOS disponible. No se cambió la configuración.\n' >&2
      exit 1
    fi
    printf 'macOS SDK: %s\n' "$orbit_sdk_path"
    xcrun --sdk macosx --show-sdk-version
    xcrun --sdk macosx --find swiftc
    xcrun --sdk macosx --find clang
    ;;
  xctest)
    # Match SwiftPM's Darwin lookup; this locates the executable, not an app or
    # a test. Successful lookup does not replace the mandatory swift test step.
    if ! xcrun --sdk macosx --find xctest; then
      orbit_found="$(orbit_developer_candidates | head -n 1 || true)"
      if [[ -n "$orbit_found" ]]; then
        # An Xcode with the runner is already installed and simply is not the
        # active developer directory. Say which one, and say the exact variable
        # that points one job at it, without selecting anything here.
        printf '::error::XCTest no está en el directorio de desarrollo activo, pero hay un Xcode instalado que sí lo trae: %s. Ejecuta las pruebas con DEVELOPER_DIR apuntando ahí; este script no cambia la selección global.\n' "$orbit_found" >&2
      else
        printf '::error::XCTest no está disponible en el entorno de desarrollo activo y no hay ningún Xcode instalado que lo traiga (se han mirado %s). Hace falta un Xcode completo con Contents/Developer/usr/bin/xctest. No se omiten las pruebas y este script no instala ni selecciona herramientas.\n' "${ORBIT_XCODE_SEARCH_PATHS-/Applications:$HOME/Applications}" >&2
      fi
      exit 1
    fi
    ;;
  *)
    printf 'Modo desconocido: usa sdk o xctest.\n' >&2
    exit 64
    ;;
esac
