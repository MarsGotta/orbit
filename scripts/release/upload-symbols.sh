#!/usr/bin/env bash
# Sends the symbols of a release's app binary (Contents/MacOS/Orbit, with the
# daemon inside since SIM-51) to Sentry (project orbit-app), so a panic report
# of that release names its functions and lines. The shipped binary is
# stripped; its dSYM (mac-build.sh) is matched by UUID. Run after
# fetch-build.sh, from the machine where the `sentry` CLI is logged in:
#
#   ORBIT_RELEASE_DIR=/ruta/absoluta bash scripts/release/upload-symbols.sh X.Y.Z
set -euo pipefail
fail() { printf 'upload-symbols: %s\n' "$*" >&2; exit 1; }

version=${1:-}
[[ $# -eq 1 && $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "versión esperada X.Y.Z, encontrada '$version'"
[[ ${ORBIT_RELEASE_DIR:-} = /* ]] || fail 'ORBIT_RELEASE_DIR debe ser una ruta absoluta'
symbols="$ORBIT_RELEASE_DIR/$version/orbit-desktop.dSYM.zip"
[[ -f $symbols ]] || fail "no está $symbols: trae la versión con fetch-build.sh"
: "${SENTRY_ORG:?falta SENTRY_ORG}"
: "${SENTRY_PROJECT:?falta SENTRY_PROJECT}"
export SENTRY_ORG SENTRY_PROJECT
command -v sentry >/dev/null || fail 'falta el CLI «sentry» en el PATH'
sentry debug-files upload --type dsym --wait "$symbols"
