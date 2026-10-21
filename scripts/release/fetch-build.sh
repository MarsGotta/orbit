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
release_connection
if [[ ! ${ORBIT_RELEASE_DIR:-} = /* ]]; then
  echo "ORBIT_RELEASE_DIR is missing: an absolute directory for version='$version', commit='$commit'." >&2
  exit 2
fi
destination="$ORBIT_RELEASE_DIR/$version"
if [[ -e $destination ]]; then
  echo "The folder of version='$version', commit='$commit' already exists: $destination" >&2
  exit 1
fi
mkdir -p -- "$ORBIT_RELEASE_DIR"
staging="$(mktemp -d "$ORBIT_RELEASE_DIR/.orbit-$version.XXXXXXXX")"
trap 'rm -rf -- "$staging"' EXIT
archive="Orbit-$version-macos-arm64.zip"
# orbit-desktop.dSYM.zip: the symbols of the app binary (MacOS/Orbit, the daemon
# inside it), for Sentry (upload-symbols.sh); never published.
for name in "$archive" "$archive.sha256" build-manifest.txt orbit-desktop.dSYM.zip; do
  if ! rsync -az -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" \
    "$ORBIT_MAC_HOST:~/orbit-rel-$version/artifacts/out/$name" "$staging/$name"; then
    echo "Could not bring '$name' from the Mac: version='$version', commit='$commit'." >&2
    exit 1
  fi
done
actual="$(shasum -a 256 "$staging/$archive" | cut -d ' ' -f 1)"
declared="$(cut -d ' ' -f 1 "$staging/$archive.sha256")"
if [[ $actual != "$declared" ]]; then
  echo "SHA-256 differs: version='$version', commit='$commit', measured='$actual', declared='$declared'." >&2
  exit 1
fi
manifest_version="$(sed -n 's/^version=//p' "$staging/build-manifest.txt")"
manifest_commit="$(sed -n 's/^source_commit=//p' "$staging/build-manifest.txt")"
manifest_dirty="$(sed -n 's/^source_dirty=//p' "$staging/build-manifest.txt")"
if [[ $manifest_version != "$version" || $manifest_commit != "$commit" || $manifest_dirty != false ]]; then
  echo "Manifest differs: version asked='$version', manifest='$manifest_version'; commit asked='$commit', manifest='$manifest_commit'; source_dirty expected='false', manifest='$manifest_dirty'." >&2
  exit 1
fi
# Verify with the verifier and version declaration from the requested commit,
# not this checkout, which may already have advanced to the next release (a
# commit from before scripts/ was reorganised keeps the verifier at
# scripts/verify-macos-archive.py and is verified from that checkout). The
# whole tree of that commit is extracted: which files its verifier reads is the
# verifier's decision, and a fixed list here would be a second, drifting copy
# of it.
mkdir -p -- "$staging/source"
git -C "$repo" archive "$commit" | tar -x -C "$staging/source"
if ! python3 -B "$staging/source/scripts/release/verify-macos-archive.py" "$staging/$archive" \
  --architecture arm64 --commit "$commit" --checksum-file "$staging/$archive.sha256" \
  --manifest "$staging/build-manifest.txt"; then
  echo "verify-macos-archive.py refused version='$version', commit='$commit'." >&2
  exit 1
fi
if ! ssh "${RELEASE_SSH_OPTIONS[@]}" -i "$ORBIT_MAC_SSH_KEY" "$ORBIT_MAC_HOST" \
  "bash \"\$HOME/orbit-rel-$version/repo/scripts/release/mac-verify.sh\" \"\$HOME/orbit-rel-$version/artifacts/out/$archive\"" \
  > "$staging/verificacion.txt" 2>&1; then
  cat -- "$staging/verificacion.txt" >&2
  echo "The physical verification failed: version='$version', commit='$commit'." >&2
  exit 1
fi
rm -rf -- "$staging/source"
mv -- "$staging" "$destination"
trap - EXIT
echo "Verified version='$version', commit='$commit': $destination/verificacion.txt"
