#!/usr/bin/env bash
set -euo pipefail

release_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$release_dir/../.." && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$release_dir/lib.sh"
release_test_tools
version=${1:-}
commit=${2:-}
release_source_commit
declared="$(git -C "$repo" show "$commit:Cargo.toml" | sed -n 's/^version = "\([^"]*\)"/\1/p' | head -1)"
if [[ $declared != "$version" ]]; then
  echo "Different version in Cargo.toml: asked='$version', declared='$declared', commit='$commit'." >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT
git clone --quiet --no-hardlinks --no-checkout "$repo" "$staging/repo"
git -C "$staging/repo" checkout --quiet --detach "$commit"
if ! check_output="$(cd "$staging/repo" && python3 -B scripts/release/set-version.py --check 2>&1)"; then
  echo "set-version.py --check refused version='$version', commit='$commit': $check_output" >&2
  exit 1
fi
release_connection
if [[ ! ${ORBIT_CUA_DRIVER_BINARY:-} =~ ^/[A-Za-z0-9_./-]+$ ]]; then
  echo 'ORBIT_CUA_DRIVER_BINARY is missing: the absolute path of the driver on the Mac.' >&2
  exit 2
fi
if [[ ! ${ORBIT_SIGNING_ENV:-} =~ ^/[A-Za-z0-9_./-]+$ ]]; then
  echo 'ORBIT_SIGNING_ENV is missing: the absolute path of the private file on the Mac.' >&2
  exit 2
fi
# The Sentry address official builds carry. It never lives in the
# repository: it is read from ORBIT_SENTRY_DSN_FILE (by default
# ~/.orbit/sentry/orbit-app.dsn on this machine) and reaches the Mac as a
# private file beside the build, never on a command line. An empty
# ORBIT_SENTRY_DSN_FILE builds without it: no switch, and nothing can leave.
sentry_file=${ORBIT_SENTRY_DSN_FILE-$HOME/.orbit/sentry/orbit-app.dsn}
if [[ -n $sentry_file && ( ! -f $sentry_file || -L $sentry_file ) ]]; then
  echo "The Sentry address file does not exist: ORBIT_SENTRY_DSN_FILE='$sentry_file'. Leave it empty to build without error reports." >&2
  exit 2
fi
remote="\$HOME/orbit-rel-$version"
if ! ssh "${RELEASE_SSH_OPTIONS[@]}" -i "$ORBIT_MAC_SSH_KEY" "$ORBIT_MAC_HOST" "test ! -e \"$remote\" && mkdir -- \"$remote\""; then
  echo "The build folder already exists or could not be created on the Mac: version='$version', commit='$commit', folder='~/orbit-rel-$version'." >&2
  exit 1
fi
# The clone contains only tracked files at this commit and its .git directory.
# There are no excludes: even .git/refs/heads/main must reach the Mac.
if ! rsync -az --delete -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" "$staging/repo/" "$ORBIT_MAC_HOST:~/orbit-rel-$version/repo/"; then
  echo "Could not send the commit to the Mac: version='$version', commit='$commit'." >&2
  exit 1
fi
remote_sentry=
if [[ -n $sentry_file ]]; then
  if ! rsync -a --chmod=F600 -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" "$sentry_file" "$ORBIT_MAC_HOST:~/orbit-rel-$version/sentry.dsn"; then
    echo "Could not send the Sentry address to the Mac: version='$version', commit='$commit'." >&2
    exit 1
  fi
  remote_sentry="$remote/sentry.dsn"
fi
# Build-time endpoints follow the selected source to the Mac. Bash's quoted
# arguments also keep URL punctuation out of the remote shell's syntax.
printf -v update_origin '%q' "${ORBIT_UPDATE_BASE_URL:-}"
printf -v pairing_hub '%q' "${VITE_ORBIT_HUB_URL:-}"
# The remote command fixes the host build's PATH first: a non-interactive SSH
# session has no /opt/homebrew/bin, and mac-build.sh re-exports it anyway.
if ! ssh "${RELEASE_SSH_OPTIONS[@]}" -i "$ORBIT_MAC_SSH_KEY" "$ORBIT_MAC_HOST" \
  "export PATH=\"$RELEASE_HOST_PATH\" && cd \"$remote/repo\" && nohup env ORBIT_UPDATE_BASE_URL=$update_origin VITE_ORBIT_HUB_URL=$pairing_hub ORBIT_BUILDDIR=\"$remote/artifacts\" ORBIT_CUA_DRIVER_BINARY='$ORBIT_CUA_DRIVER_BINARY' ORBIT_SIGNING_ENV='$ORBIT_SIGNING_ENV' ORBIT_SENTRY_DSN_FILE=\"$remote_sentry\" bash scripts/release/mac-build.sh > \"$remote/build.log\" 2>&1 < /dev/null &"; then
  echo "Could not launch the build on the Mac: version='$version', commit='$commit'." >&2
  exit 1
fi
echo "Build launched: version='$version', commit='$commit'; log at $ORBIT_MAC_HOST:~/orbit-rel-$version/build.log"
