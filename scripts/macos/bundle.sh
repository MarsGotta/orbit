#!/usr/bin/env bash
set -euo pipefail

# shellcheck source=scripts/release/lib.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/../release/lib.sh"
release_test_tools

# Run on the target Mac architecture. No credentials are written into the bundle.
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Orbit's package is built on macOS 15+ with Xcode 16+." >&2
  exit 1
fi
orbit_repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
# The release input is staged explicitly by the builder. A signed package must
# never fetch executable code while it is being assembled.
orbit_cua_driver="${ORBIT_CUA_DRIVER_BINARY:-}"
if [[ -z "$orbit_cua_driver" ]]; then
  echo "ORBIT_CUA_DRIVER_BINARY is missing: give the absolute path of the published cua-driver." >&2
  exit 1
fi
if [[ "$orbit_cua_driver" != /* ]]; then
  echo "ORBIT_CUA_DRIVER_BINARY must be an absolute path." >&2
  exit 1
fi
if ! [[ -f "$orbit_cua_driver" && -x "$orbit_cua_driver" ]]; then
  echo "The CUA driver does not exist or is not executable: $orbit_cua_driver" >&2
  exit 1
fi
orbit_cua_expected_version="$(sed -n 's/^pub const VERIFIED_DRIVER_VERSION: &str = "\([0-9][0-9.]*\)";/\1/p' "$orbit_repo/crates/orbit-daemon/src/cua.rs")"
if [[ ! "$orbit_cua_expected_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "VERIFIED_DRIVER_VERSION could not be read from crates/orbit-daemon/src/cua.rs." >&2
  exit 1
fi
if ! orbit_cua_version_output="$("$orbit_cua_driver" --version 2>&1)"; then
  echo "'$orbit_cua_driver --version' could not be run." >&2
  exit 1
fi
orbit_cua_reported_version="$(printf '%s\n' "$orbit_cua_version_output" | sed -nE 's/^([^[:space:]]+[[:space:]]+)?v?([0-9]+\.[0-9]+\.[0-9]+)$/\2/p')"
if [[ "$orbit_cua_reported_version" != "$orbit_cua_expected_version" ]]; then
  echo "cua-driver version '$orbit_cua_version_output'; expected '$orbit_cua_expected_version'." >&2
  exit 1
fi
# Read from the workspace manifest instead of repeating the number here. The
# version lived in three hand-written places, and a package whose name disagrees
# with the binary inside it cannot be traced back to what built it.
orbit_default_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$orbit_repo/Cargo.toml" | head -1)"
orbit_version="${ORBIT_VERSION:-$orbit_default_version}"
if [[ ! "$orbit_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "ORBIT_VERSION must have the numeric form major.minor.patch." >&2
  exit 1
fi
orbit_identity="${ORBIT_SIGN_IDENTITY:--}"
# A signing identity that persists across builds is what keeps macOS treating
# Orbit as the same application. Signing ad-hoc makes the designated requirement
# the binary's own cdhash, which changes every build, so every TCC grant the
# person made is attached to a version that no longer exists.
orbit_keychain="${ORBIT_SIGN_KEYCHAIN:-}"
orbit_keychain_password_file="${ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE:-}"
# Notarization: Apple checks the package and staples the result to the app, so a
# person who downloads it never meets macOS's "cannot be verified" refusal. All
# three values are required together; a key without an issuer would fail late,
# after the whole build, instead of here.
orbit_notary_key="${ORBIT_NOTARY_KEY:-}"
orbit_notary_key_id="${ORBIT_NOTARY_KEY_ID:-}"
orbit_notary_issuer="${ORBIT_NOTARY_ISSUER:-}"
orbit_notarize=false
if [[ -n "$orbit_notary_key" || -n "$orbit_notary_key_id" || -n "$orbit_notary_issuer" ]]; then
  if [[ -z "$orbit_notary_key" || -z "$orbit_notary_key_id" || -z "$orbit_notary_issuer" ]]; then
    echo "Partial notarization settings: ORBIT_NOTARY_KEY, ORBIT_NOTARY_KEY_ID and ORBIT_NOTARY_ISSUER are needed together." >&2
    exit 1
  fi
  [[ -f "$orbit_notary_key" ]] || { echo "The notarization key $orbit_notary_key does not exist" >&2; exit 1; }
  # Apple refuses to notarize an ad-hoc signature, and would say so only after
  # uploading the whole archive.
  [[ "$orbit_identity" != "-" ]] || { echo "An ad-hoc signature cannot be notarized." >&2; exit 1; }
  orbit_notarize=true
fi
orbit_output="${ORBIT_OUTPUT_DIR:-$orbit_repo/dist}"
if [[ "$orbit_output" != /* ]]; then orbit_output="$PWD/$orbit_output"; fi
# ORBIT_PRUEBAS=1 builds «Orbit Pruebas», the copy the real test on the
# Mac installs beside Orbit (docs/self-hosting.md): its own bundle
# name, identifier, data directory and Keychain service (the Cargo feature
# orbit-core/pruebas, see crates/orbit-core/src/orbit_bundle.rs). Without it
# this script builds exactly what it always built.
orbit_name=Orbit
orbit_identifier=app.orbit.companion
orbit_features=()
if [[ "${ORBIT_PRUEBAS:-}" == 1 ]]; then
  orbit_name="Orbit Pruebas"
  orbit_identifier=app.orbit.companion.pruebas
  orbit_features=(--features orbit-core/pruebas)
fi
orbit_bundle="$orbit_output/$orbit_name.app"
CUA_ENGINE_RELATIVE_PATH="Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
orbit_engine_binary="$orbit_bundle/Contents/$CUA_ENGINE_RELATIVE_PATH"
orbit_engine_bundle="${orbit_engine_binary%/Contents/MacOS/cua-driver}"
orbit_arch="$(uname -m)"
# The Rust target of this Mac, for the crates the third-party notice lists.
case "$orbit_arch" in
  arm64) orbit_rust_target=aarch64-apple-darwin ;;
  x86_64) orbit_rust_target=x86_64-apple-darwin ;;
  *) echo "Unknown Mac architecture: $orbit_arch" >&2; exit 1 ;;
esac
orbit_archive="$orbit_output/Orbit-$orbit_version-macos-$orbit_arch.zip"
if [[ -e "$orbit_bundle" || -e "$orbit_archive" ]]; then
  echo "A package already exists in $orbit_output. Use ORBIT_OUTPUT_DIR for a new build." >&2
  exit 1
fi
cd -- "$orbit_repo"
swift test --package-path native/macos
pnpm --dir apps/desktop install --frozen-lockfile
# The daemon runs inside the app, so this one build is the daemon's
# too, with the same features; the separate daemon binary is no longer shipped.
pnpm --dir apps/desktop tauri build --no-bundle ${orbit_features[@]+"${orbit_features[@]}"}
swift build --package-path native/macos --configuration release
orbit_swift_bin="$(swift build --package-path native/macos --configuration release --show-bin-path)"
mkdir -p -- \
  "$orbit_bundle/Contents/MacOS" \
  "$orbit_engine_bundle/Contents/MacOS" \
  "$orbit_bundle/Contents/Resources/third-party/cua-driver"
cp -- target/release/orbit-desktop "$orbit_bundle/Contents/MacOS/Orbit"
cp -- "$orbit_swift_bin/OrbitNative" "$orbit_bundle/Contents/MacOS/OrbitNative"
cp -- "$orbit_cua_driver" "$orbit_engine_binary"
cp -- apps/desktop/src-tauri/icons/icon.icns "$orbit_bundle/Contents/Resources/icon.icns"
cp -- native/macos/Info.plist "$orbit_bundle/Contents/Info.plist"
cp -- LICENSE "$orbit_bundle/Contents/Resources/LICENSE"
cp -- third-party/cua-driver/LICENSE.md \
  "$orbit_bundle/Contents/Resources/third-party/cua-driver/LICENSE.md"
# The licences of what others wrote and the app carries: the interface's
# packages as pnpm installed them for this build, and the crates compiled into
# the app, read from this checkout's lockfiles offline: nothing is fetched.
orbit_notice_input="$(mktemp -d)"
pnpm --dir apps/desktop licenses list --prod --json > "$orbit_notice_input/pnpm.json"
cargo metadata --format-version 1 --locked --offline --filter-platform "$orbit_rust_target" \
  ${orbit_features[@]+"${orbit_features[@]}"} > "$orbit_notice_input/cargo.json"
python3 -B "$orbit_repo/scripts/release/third-party-notices.py" \
  --pnpm-licenses "$orbit_notice_input/pnpm.json" \
  --cargo-metadata "$orbit_notice_input/cargo.json" \
  --root orbit-desktop \
  --output "$orbit_bundle/Contents/Resources/third-party/NOTICES.txt"
rm -rf -- "$orbit_notice_input"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $orbit_version" "$orbit_bundle/Contents/Info.plist"
if [[ "${ORBIT_PRUEBAS:-}" == 1 ]]; then
  # Its own identity for macOS: its permissions, its WebView storage and its
  # place in System Settings are never Orbit's.
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier app.orbit.companion.pruebas" "$orbit_bundle/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleName $orbit_name" "$orbit_bundle/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName $orbit_name" "$orbit_bundle/Contents/Info.plist"
fi
# The innermost app owns the engine's LaunchServices identity. The executable
# is copied unchanged; it remains a child of Orbit in embedded mode for TCC.
cat > "$orbit_engine_bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$orbit_identifier.cua-driver</string>
  <key>CFBundleName</key><string>Orbit Engine</string>
  <key>CFBundleExecutable</key><string>cua-driver</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$orbit_cua_expected_version</string>
  <key>CFBundleShortVersionString</key><string>$orbit_cua_expected_version</string>
  <key>LSUIElement</key><true/>
</dict></plist>
EOF
orbit_commit="$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"
orbit_dirty=false
if [[ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]]; then orbit_dirty=true; fi
# The manifest is sealed inside the signature, so it has to be written before
# signing and cannot be corrected afterwards. Claiming notarization up front is
# only honest because a failed notarization aborts the build: no package with
# this claim ever leaves the machine unnotarized.
printf 'version=%s\narchitecture=%s\nsource_commit=%s\nsource_dirty=%s\nminimum_macos=15.0\nnotarized=%s\n' \
  "$orbit_version" "$orbit_arch" "$orbit_commit" "$orbit_dirty" "$orbit_notarize" > "$orbit_output/build-manifest.txt"
cp -- "$orbit_output/build-manifest.txt" "$orbit_bundle/Contents/Resources/build-manifest.txt"
test "$(lipo -archs "$orbit_bundle/Contents/MacOS/OrbitNative")" = "$orbit_arch"
test "$(lipo -archs "$orbit_engine_binary")" = "$orbit_arch"
test "$(lipo -archs "$orbit_bundle/Contents/MacOS/Orbit")" = "$orbit_arch"
orbit_sign_options=(--force --options runtime --sign "$orbit_identity")
if [[ -n "$orbit_keychain" ]]; then
  # Unlocked here, and never echoed: a build over SSH has no session to answer an
  # authorisation prompt, and codesign fails with errSecInternalComponent instead
  # of saying the key was unreachable.
  if [[ -n "$orbit_keychain_password_file" ]]; then
    security unlock-keychain -p "$(cat -- "$orbit_keychain_password_file")" "$orbit_keychain" > /dev/null
  fi
  orbit_sign_options+=(--keychain "$orbit_keychain")
fi
if [[ "$orbit_identity" == "-" ]]; then
  : # Nothing to timestamp: an ad-hoc signature has no certificate behind it.
elif [[ "$orbit_notarize" == true || -z "$orbit_keychain" ]]; then
  # Apple rejects a notarization request whose signature carries no secure
  # timestamp, and it rejects it after the upload, not before.
  orbit_sign_options+=(--timestamp)
else
  # A locally issued certificate outlives any build, so a timestamp buys nothing
  # and would make every package depend on Apple's server being reachable.
  orbit_sign_options+=(--timestamp=none)
fi
codesign "${orbit_sign_options[@]}" --entitlements native/macos/Orbit.entitlements \
  "$orbit_bundle/Contents/MacOS/OrbitNative"
codesign "${orbit_sign_options[@]}" "$orbit_engine_bundle"
codesign "${orbit_sign_options[@]}" "$orbit_bundle/Contents/MacOS/Orbit"
codesign "${orbit_sign_options[@]}" --entitlements native/macos/Orbit.entitlements "$orbit_bundle"
codesign --verify --deep --strict --verbose=2 "$orbit_bundle"
python3 -B "$orbit_repo/scripts/macos/smoke-native.py" --binary "$orbit_bundle/Contents/MacOS/OrbitNative"
# The whole point of signing with an identity is a requirement that survives the
# next build. If it still names a cdhash, the identity was ignored and the
# package would silently cost the person every permission again.
if [[ "$orbit_identity" != "-" ]]; then
  orbit_requirement=""
  for orbit_requirement_target in \
    "$orbit_bundle/Contents/MacOS/OrbitNative" \
    "$orbit_engine_binary" \
    "$orbit_engine_bundle" \
    "$orbit_bundle/Contents/MacOS/Orbit"; do
    orbit_requirement="$(codesign -d -r- "$orbit_requirement_target" 2>&1 | sed -n 's/^designated => //p')"
    if [[ "$orbit_requirement" == *cdhash* || "$orbit_requirement" != *certificate* ]]; then
      echo "The signature of $orbit_requirement_target is not tied to the certificate: $orbit_requirement" >&2
      exit 1
    fi
    echo "Designated requirement of $orbit_requirement_target: $orbit_requirement"
  done
fi
if [[ "$orbit_notarize" == true ]]; then
  # Apple takes a zip to inspect, but staples its answer to the .app, so the
  # upload copy is deliberately thrown away: the archive that ships is built
  # afterwards, from the stapled bundle. Shipping this one instead is how a
  # package ends up notarized on Apple's servers and unstapled on disk.
  orbit_upload="$orbit_output/notarize-upload.zip"
  ditto -c -k --keepParent --norsrc --noextattr --noqtn "$orbit_bundle" "$orbit_upload"
  # Submitted and polled separately, on purpose. `--wait` gives up when a single
  # status request times out, and it gives up on a submission Apple already
  # accepted: measured here, a transient NSURLErrorDomain -1001 after a
  # successful upload failed the whole build and threw away the ticket.
  orbit_submission="$(
    xcrun notarytool submit "$orbit_upload" \
      --key "$orbit_notary_key" --key-id "$orbit_notary_key_id" --issuer "$orbit_notary_issuer" \
      --output-format json | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'
  )"
  rm -f -- "$orbit_upload"
  echo "Notarization submitted: $orbit_submission"
  orbit_status=""
  # Bounded by attempts rather than by a single long request, so a dropped
  # connection costs one poll instead of the build. Three hours, not thirty
  # minutes: measured here, the first submission of a freshly issued Developer ID
  # sat In Progress for over two hours while later ones took minutes, and a
  # timeout would throw away a verdict that was still coming.
  for _ in $(seq 1 720); do
    orbit_status="$(
      xcrun notarytool info "$orbit_submission" \
        --key "$orbit_notary_key" --key-id "$orbit_notary_key_id" --issuer "$orbit_notary_issuer" \
        --output-format json 2>/dev/null \
        | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))' 2>/dev/null
    )"
    [[ "$orbit_status" == "In Progress" || -z "$orbit_status" ]] || break
    sleep 15
  done
  if [[ "$orbit_status" != "Accepted" ]]; then
    echo "Apple did not accept the package (status: ${orbit_status:-unknown}). Detail:" >&2
    xcrun notarytool log "$orbit_submission" \
      --key "$orbit_notary_key" --key-id "$orbit_notary_key_id" --issuer "$orbit_notary_issuer" >&2 || true
    exit 1
  fi
  xcrun stapler staple "$orbit_bundle"
  # Measured, not assumed: stapler reports success on a bundle it did not
  # actually change if the ticket was already present from an earlier attempt.
  xcrun stapler validate "$orbit_bundle"
  # The verdict a person's Mac will reach, asked of the same service that will
  # decide it. Anything other than "accepted" here becomes a refusal to open on
  # a machine that has never seen this app.
  spctl --assess --type execute --verbose=4 "$orbit_bundle"
fi
# macOS stamps files with com.apple.provenance, and a plain ditto carries every
# extended attribute into the archive as an AppleDouble `._` companion. Those
# are metadata of the machine that built the package, not part of the app or of
# its signature, and the archive verifier refuses anything outside the built
# tree. Signature validity after extraction was checked, not assumed.
ditto -c -k --keepParent --norsrc --noextattr --noqtn "$orbit_bundle" "$orbit_archive"
(
  cd -- "$orbit_output"
  shasum -a 256 "Orbit-$orbit_version-macos-$orbit_arch.zip" > "Orbit-$orbit_version-macos-$orbit_arch.zip.sha256"
)
echo "Created: $orbit_bundle"
if [[ "$orbit_identity" == "-" ]]; then
  echo "Ad-hoc signature: macOS will treat this version as a different application and the permissions granted are not kept."
elif [[ "$orbit_notarize" == true ]]; then
  echo "Signed and notarized by Apple, with the ticket stapled: it opens without warnings on a Mac that has never seen it."
else
  echo "Signed with a stable identity; the permissions survive the update. Not notarized for public distribution."
fi
