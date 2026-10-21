#!/usr/bin/env bash
# Builds «Orbit Pruebas» on the Mac from this tree and installs it in
# ~/Applications/Orbit Pruebas.app, for the real test. Runs on the Mac, by the
# reviewer:
#
#   ORBIT_CUA_DRIVER_BINARY=/path/cua-driver ORBIT_SIGNING_ENV=/path/orbit-signing.env \
#     bash scripts/e2e-mac/build-test-app.sh
#
# The same packaging as a release (scripts/macos/bundle.sh) with ORBIT_PRUEBAS=1,
# signed with the same stable identity — so the permissions the user grants
# Orbit Pruebas once survive the next build — and never notarized: it is not
# published. Only the three signing keys are taken from the signing file; the
# notarization ones are left out on purpose. Nothing here touches Orbit.app.
set -euo pipefail

repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
# shellcheck source=scripts/release/lib.sh
source "$repo/scripts/release/lib.sh"
# A non-interactive SSH session needs the Mac's build tools on its PATH.
export PATH="${RELEASE_HOST_PATH//\$HOME/$HOME}"

destination="$HOME/Applications/Orbit Pruebas.app"

if [[ $(uname -s) != Darwin ]]; then
  echo "Orbit Pruebas is built on the Mac." >&2
  exit 1
fi
if [[ ! ${ORBIT_SIGNING_ENV:-} = /* || ! -f ${ORBIT_SIGNING_ENV:-} || -L ${ORBIT_SIGNING_ENV:-} ]]; then
  echo "ORBIT_SIGNING_ENV must be the absolute path of the signing file: '${ORBIT_SIGNING_ENV:-}'." >&2
  exit 1
fi
release_read_signing_env
for key in ORBIT_SIGN_IDENTITY ORBIT_SIGN_KEYCHAIN ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE; do
  if [[ -z ${!key:-} ]]; then
    echo "$key is missing from $ORBIT_SIGNING_ENV." >&2
    exit 1
  fi
done
unset ORBIT_NOTARY_KEY ORBIT_NOTARY_KEY_ID ORBIT_NOTARY_ISSUER

output="$(mktemp -d)"
trap 'rm -rf -- "$output"' EXIT
ORBIT_PRUEBAS=1 ORBIT_OUTPUT_DIR="$output/out" bash "$repo/scripts/macos/bundle.sh"

identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$output/out/Orbit Pruebas.app/Contents/Info.plist")"
if [[ $identifier != app.orbit.companion.pruebas ]]; then
  echo "The bundle built is «$identifier», not Orbit Pruebas: it is not installed." >&2
  exit 1
fi
if ps -axo comm= | grep -qF -- "$destination/Contents/"; then
  echo "Orbit Pruebas is open; quit it before installing the new one." >&2
  exit 1
fi
# Only this exact path, which is Orbit Pruebas's own.
rm -rf -- "$destination"
mkdir -p -- "$HOME/Applications"
ditto "$output/out/Orbit Pruebas.app" "$destination"
codesign --verify --deep --strict "$destination"
echo "Installed: $destination ($identifier)"
