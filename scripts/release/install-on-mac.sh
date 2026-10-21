#!/bin/bash
# Install the published Orbit on this Mac, as the terminal's update did before
# the command line was retired. Runs on the Mac; over SSH:
#
#   ssh "$ORBIT_MAC_HOST" 'bash -s' < scripts/release/install-on-mac.sh
#
# Checks the published install.sh against the installer_sha256 of latest.txt
# before touching anything, then asks Orbit to quit and runs the installer with
# --relaunch, which waits for Orbit to leave and opens the new one.
set -euo pipefail

: "${ORBIT_INSTALL_BASE:?ORBIT_INSTALL_BASE is missing}"
base="$ORBIT_INSTALL_BASE"
bundle_id="app.orbit.companion"

fail() {
  printf 'install-on-mac: %s\n' "$1" >&2
  exit 1
}

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

curl -fsSL --max-time 60 "$base/latest.txt" -o "$work/latest.txt" \
  || fail "could not download $base/latest.txt"
expected="$(sed -n 's/^installer_sha256=//p' "$work/latest.txt" | head -n 1)"
[[ "$expected" =~ ^[0-9a-f]{64}$ ]] \
  || fail "$base/latest.txt has no 64-hex installer_sha256 (it has '$expected')"
curl -fsSL --max-time 60 "$base/install.sh" -o "$work/install.sh" \
  || fail "could not download $base/install.sh"
seen="$(shasum -a 256 "$work/install.sh" | cut -d' ' -f1)"
[[ "$seen" == "$expected" ]] \
  || fail "install.sh does not match installer_sha256: expected $expected; received $seen. Nothing is installed."

# Asked only if it is running: telling a closed app to quit would launch it.
osascript -e "if application id \"$bundle_id\" is running then tell application id \"$bundle_id\" to quit"
ORBIT_INSTALL_BASE="$base" /bin/bash "$work/install.sh" --relaunch
