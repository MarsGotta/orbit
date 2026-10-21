#!/usr/bin/env bash
# Sends the symbols of a release's app binary (Contents/MacOS/Orbit, with the
# daemon inside) to Sentry (project orbit-app), so a panic report
# of that release names its functions and lines. The shipped binary is
# stripped; its dSYM (mac-build.sh) is matched by UUID. Run after
# fetch-build.sh, from the machine where the `sentry` CLI is logged in:
#
#   ORBIT_RELEASE_DIR=/absolute/path bash scripts/release/upload-symbols.sh X.Y.Z
set -euo pipefail
fail() { printf 'upload-symbols: %s\n' "$*" >&2; exit 1; }

version=${1:-}
[[ $# -eq 1 && $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version expected X.Y.Z, found '$version'"
[[ ${ORBIT_RELEASE_DIR:-} = /* ]] || fail 'ORBIT_RELEASE_DIR must be an absolute path'
symbols="$ORBIT_RELEASE_DIR/$version/orbit-desktop.dSYM.zip"
[[ -f $symbols ]] || fail "$symbols is not there: bring the version with fetch-build.sh"
: "${SENTRY_ORG:?SENTRY_ORG is missing}"
: "${SENTRY_PROJECT:?SENTRY_PROJECT is missing}"
export SENTRY_ORG SENTRY_PROJECT
command -v sentry >/dev/null || fail "the 'sentry' CLI is missing from the PATH"
sentry debug-files upload --type dsym --wait "$symbols"
