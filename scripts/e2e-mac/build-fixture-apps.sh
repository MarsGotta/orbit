#!/usr/bin/env bash
# Builds the real test's two helper apps on the Mac and installs them in
# ~/Applications, signed with Orbit Pruebas's stable identity so the
# permission granted to one survives the next build. Never notarized.
#
# - «Orbit Pruebas Teclas» (keys.swift): the only sender of real keys.
#   Launched with `open`, it is its own responsible process, so the
#   Accessibility permission is this app's and never the SSH session's. It
#   posts keys only to Orbit Pruebas, frontmost.
# - «Orbit Pruebas Remote» (remote-speaker.swift, remote-route.swift): the
#   silent far side of a meeting for check 25. It plays only into BlackHole
#   16ch, and only the test build offers it as an application source.
#
#   ORBIT_SIGNING_ENV=/path/orbit-signing.env bash scripts/e2e-mac/build-fixture-apps.sh
set -euo pipefail

repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
# shellcheck source=scripts/release/lib.sh
source "$repo/scripts/release/lib.sh"
export PATH="${RELEASE_HOST_PATH//\$HOME/$HOME}"

if [[ $(uname -s) != Darwin ]]; then
  echo "The real test's helper apps are built on the Mac." >&2
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
# Unlocked here, and never echoed: a build over SSH has no session to answer an
# authorisation prompt (scripts/macos/bundle.sh does the same).
security unlock-keychain -p "$(cat -- "$ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE")" "$ORBIT_SIGN_KEYCHAIN" > /dev/null

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

# build_app NAME IDENTIFIER EXECUTABLE SOURCE...: the first source is the entry point.
build_app() {
  local name=$1 identifier=$2 executable=$3
  shift 3
  local bundle="$work/$name.app" destination="$HOME/Applications/$name.app" sources=()
  mkdir -p -- "$bundle/Contents/MacOS" "$work/$identifier"
  # Swift's multi-file executable entry point is named main.swift.
  cp -- "$1" "$work/$identifier/main.swift"
  sources=("$work/$identifier/main.swift" "${@:2}")
  swiftc -O -o "$bundle/Contents/MacOS/$executable" "${sources[@]}"
  cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>$executable</string>
  <key>CFBundleIdentifier</key><string>$identifier</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
</dict>
</plist>
PLIST
  codesign --force --options runtime --timestamp=none --keychain "$ORBIT_SIGN_KEYCHAIN" \
    --sign "$ORBIT_SIGN_IDENTITY" "$bundle"
  codesign --verify --strict "$bundle"
  # Only this exact path, which is the helper's own.
  rm -rf -- "$destination"
  mkdir -p -- "$HOME/Applications"
  ditto "$bundle" "$destination"
  echo "Installed: $destination ($identifier)"
}

e2e="$repo/scripts/e2e-mac"
build_app "Orbit Pruebas Teclas" app.orbit.companion.pruebas.keys OrbitPruebasTeclas "$e2e/keys.swift"
build_app "Orbit Pruebas Remote" app.orbit.pruebas.remote RemoteSpeaker \
  "$e2e/remote-speaker.swift" "$e2e/remote-route.swift"
