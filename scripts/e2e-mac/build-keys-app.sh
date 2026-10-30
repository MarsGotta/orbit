#!/usr/bin/env bash
# Builds «Orbit Pruebas Teclas» on the Mac and installs it in
# ~/Applications/Orbit Pruebas Teclas.app. It is the real test's only sender of
# real keys (keys.swift): launched with `open`, it is its own responsible
# process, so the Accessibility permission the owner grants is this app's and
# never the SSH session's. It posts keys only to Orbit Pruebas, frontmost.
#
#   ORBIT_SIGNING_ENV=/path/orbit-signing.env bash scripts/e2e-mac/build-keys-app.sh
#
# Signed with Orbit Pruebas's stable identity, so the permission survives the
# next build. Never notarized: it is not published.
set -euo pipefail

repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
# shellcheck source=scripts/release/lib.sh
source "$repo/scripts/release/lib.sh"
export PATH="${RELEASE_HOST_PATH//\$HOME/$HOME}"

destination="$HOME/Applications/Orbit Pruebas Teclas.app"
identifier=app.orbit.companion.pruebas.keys

if [[ $(uname -s) != Darwin ]]; then
  echo "Orbit Pruebas Teclas is built on the Mac." >&2
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

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
bundle="$work/Orbit Pruebas Teclas.app"
mkdir -p -- "$bundle/Contents/MacOS"
swiftc -O -o "$bundle/Contents/MacOS/OrbitPruebasTeclas" "$repo/scripts/e2e-mac/keys.swift"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>OrbitPruebasTeclas</string>
  <key>CFBundleIdentifier</key><string>$identifier</string>
  <key>CFBundleName</key><string>Orbit Pruebas Teclas</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
</dict>
</plist>
PLIST
# Unlocked here, and never echoed: a build over SSH has no session to answer an
# authorisation prompt (scripts/macos/bundle.sh does the same).
security unlock-keychain -p "$(cat -- "$ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE")" "$ORBIT_SIGN_KEYCHAIN" > /dev/null
codesign --force --options runtime --timestamp=none --keychain "$ORBIT_SIGN_KEYCHAIN" \
  --sign "$ORBIT_SIGN_IDENTITY" "$bundle"
codesign --verify --strict "$bundle"
# Only this exact path, which is the key sender's own.
rm -rf -- "$destination"
mkdir -p -- "$HOME/Applications"
ditto "$bundle" "$destination"
echo "Installed: $destination ($identifier)"
