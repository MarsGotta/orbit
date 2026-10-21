#!/usr/bin/env bash
set -euo pipefail

release_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$release_dir/../.." && pwd -P)"
# shellcheck source=scripts/release/lib.sh
source "$release_dir/lib.sh"
# A non-interactive SSH session on the Mac arrives with
# PATH=~/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin: no pnpm and no node (both
# live in /opt/homebrew/bin), bash 3.2 instead of 5.x, python3 3.9 instead of
# 3.13. This is the PATH the build Mac's own build script used, verbatim.
export PATH="${RELEASE_HOST_PATH//\$HOME/$HOME}"

release_test_tools

export ORBIT_ARCH=arm64
if [[ $(uname -s) != Darwin || $(uname -m) != "$ORBIT_ARCH" ]]; then
  echo "Orbit's build requires macOS $ORBIT_ARCH." >&2
  exit 1
fi
if [[ ! ${ORBIT_BUILDDIR:-} = /* || -e ${ORBIT_BUILDDIR:-} || -L ${ORBIT_BUILDDIR:-} ]]; then
  echo "ORBIT_BUILDDIR must be the absolute path of an artifacts folder that does not exist yet: '${ORBIT_BUILDDIR:-}'. This version's temp/ and out/ are created there." >&2
  exit 1
fi

# The Sentry address, from a private file outside the repository,
# given only to the compilation of the bundle (`option_env!` in the daemon).
# Its value is never printed. Without the file the build has no error
# reports: the switch is not offered.
sentry_dsn=
if [[ -n ${ORBIT_SENTRY_DSN_FILE:-} ]]; then
  release_private_file "$ORBIT_SENTRY_DSN_FILE" ORBIT_SENTRY_DSN_FILE
  sentry_file=$RELEASE_PRIVATE_FILE
  IFS= read -r sentry_dsn < "$sentry_file" || true
  if [[ ! $sentry_dsn =~ ^https://[0-9a-f]{32}@[A-Za-z0-9.-]+/[0-9]+$ ]]; then
    echo "ORBIT_SENTRY_DSN_FILE does not contain a Sentry address (https://key@server/project on one line): $sentry_file" >&2
    exit 1
  fi
fi

if [[ -n ${ORBIT_SIGNING_ENV:-} ]]; then
  release_read_signing_env
fi
for key in ORBIT_SIGN_IDENTITY ORBIT_SIGN_KEYCHAIN ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE ORBIT_NOTARY_KEY ORBIT_NOTARY_KEY_ID ORBIT_NOTARY_ISSUER; do
  if [[ -z ${!key:-} ]]; then
    echo "$key is missing to sign and notarize the version." >&2
    exit 1
  fi
done
if [[ $ORBIT_SIGN_IDENTITY == - ]]; then
  echo 'ORBIT_SIGN_IDENTITY cannot be ad-hoc in a notarized build.' >&2
  exit 1
fi
# The signing material is proven before anything compiles: a keychain without
# the certificate would otherwise fail at codesign, half an hour into the
# build, after fmt, the tests and clippy have already run.
if [[ ! -f $ORBIT_SIGN_KEYCHAIN ]]; then
  echo "The signing keychain ORBIT_SIGN_KEYCHAIN does not exist: '$ORBIT_SIGN_KEYCHAIN'." >&2
  exit 1
fi
if [[ ! -f $ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE ]]; then
  echo "ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE does not exist: '$ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE'." >&2
  exit 1
fi
if [[ ! -f $ORBIT_NOTARY_KEY ]]; then
  echo "The notarization key ORBIT_NOTARY_KEY does not exist: '$ORBIT_NOTARY_KEY'." >&2
  exit 1
fi
if ! security find-certificate -c "$ORBIT_SIGN_IDENTITY" "$ORBIT_SIGN_KEYCHAIN" > /dev/null; then
  echo "The keychain '$ORBIT_SIGN_KEYCHAIN' finds no certificate for the identity '$ORBIT_SIGN_IDENTITY'." >&2
  exit 1
fi

mkdir -p -- "$ORBIT_BUILDDIR/temp" "$ORBIT_BUILDDIR/out"
export RUNNER_TEMP="$ORBIT_BUILDDIR/temp"
export ORBIT_OUTPUT_DIR="$ORBIT_BUILDDIR/out"
cd -- "$repo"

# The stages of the build, in order, each stopping it when it fails.
bash scripts/macos/check-sdk.sh sdk
rustup show

cargo fmt --all -- --check
# orbit-app-core tests the daemon binary for freshness: it has to
# exist before the tests run, as in scripts/check.sh. The 0.3.13 build
# stopped here on 2026-09-29 without this line.
cargo build -p orbit-daemon --bin orbit --locked
# macOS gives a process 256 open files by default (`launchctl limit maxfiles`);
# Linux, 524288. The daemon's tests run many runtimes at once in one process,
# and each holds its writer and three WAL readers: on the MacBook
# on 2026-10-02, `cargo test -p orbit-daemon --lib` failed 12 and 5 tests with
# EMFILE at 256 and passed 1353/1353 twice at 4096. The bound is the test
# process's, not the daemon's: a running daemon holds one runtime.
ulimit -n 4096
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings

bash scripts/macos/check-sdk.sh xctest
swift test --package-path native/macos

cc -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -pthread -I native/macos/Sources/OrbitAudioRing/include native/macos/Sources/OrbitAudioRing/OrbitAudioRing.c native/macos/Tests/ring_test.c -o "$RUNNER_TEMP/orbit-ring-test" && "$RUNNER_TEMP/orbit-ring-test"
# stop_barrier_test.c includes OrbitAudioRing.c itself (line 4): linking it
# again is a duplicate definition.
cc -std=c11 -Wall -Wextra -Werror -fsanitize=address,undefined -pthread -I native/macos/Sources/OrbitAudioRing/include native/macos/Tests/stop_barrier_test.c -o "$RUNNER_TEMP/orbit-stop-barrier-test" && "$RUNNER_TEMP/orbit-stop-barrier-test"

if [[ -n $sentry_dsn ]]; then
  ORBIT_SENTRY_DSN=$sentry_dsn bash scripts/macos/bundle.sh
else
  bash scripts/macos/bundle.sh
fi

commit="$(git rev-parse HEAD)"
bash scripts/release/verify-built-archive.sh "$ORBIT_OUTPUT_DIR" "$commit" "$ORBIT_ARCH"

# The app's line tables (Cargo.toml [profile.release]): a dSYM beside the
# stripped binary, never in the bundle. The daemon runs inside the app, so
# the binary that panics is Contents/MacOS/Orbit, which scripts/macos/bundle.sh
# copies from target/release/orbit-desktop (`tauri build`); Cargo leaves its
# dSYM beside it. It goes to Sentry with the release
# (scripts/release/upload-symbols.sh), so it must be the bundled binary's:
# same UUID (signing does not change it).
dsym=target/release/orbit-desktop.dSYM
binary="$ORBIT_OUTPUT_DIR/Orbit.app/Contents/MacOS/Orbit"
if [[ ! -d $dsym ]]; then
  echo "$dsym is missing: the app's build did not leave its symbols." >&2
  exit 1
fi
binary_uuid="$(dwarfdump --uuid "$binary" | awk '{print $2}')"
dsym_uuid="$(dwarfdump --uuid "$dsym" | awk '{print $2}')"
if [[ -z $binary_uuid || $binary_uuid != "$dsym_uuid" ]]; then
  echo "The app's dSYM ($dsym_uuid) is not its binary's, $binary ($binary_uuid)." >&2
  exit 1
fi
ditto -c -k --keepParent "$dsym" "$ORBIT_OUTPUT_DIR/orbit-desktop.dSYM.zip"
ls -l "$ORBIT_OUTPUT_DIR"/*.zip
shasum -a 256 "$ORBIT_OUTPUT_DIR"/*.zip
