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
  echo "Falta ORBIT_RELEASE_DIR: directorio absoluto para versión='$version', commit='$commit'." >&2
  exit 2
fi
destination="$ORBIT_RELEASE_DIR/$version"
if [[ -e $destination ]]; then
  echo "Ya existe la carpeta de versión='$version', commit='$commit': $destination" >&2
  exit 1
fi
mkdir -p -- "$ORBIT_RELEASE_DIR"
staging="$(mktemp -d "$ORBIT_RELEASE_DIR/.orbit-$version.XXXXXXXX")"
trap 'rm -rf -- "$staging"' EXIT
archive="Orbit-$version-macos-arm64.zip"
# orbit-desktop.dSYM.zip: the symbols of the app binary (MacOS/Orbit, the daemon
# inside it since SIM-51), for Sentry (upload-symbols.sh); never published.
for name in "$archive" "$archive.sha256" build-manifest.txt orbit-desktop.dSYM.zip; do
  if ! rsync -az -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" \
    "$ORBIT_MAC_HOST:~/orbit-rel-$version/artifacts/out/$name" "$staging/$name"; then
    echo "No se pudo traer '$name' del Mac: versión='$version', commit='$commit'." >&2
    exit 1
  fi
done
actual="$(shasum -a 256 "$staging/$archive" | cut -d ' ' -f 1)"
declared="$(cut -d ' ' -f 1 "$staging/$archive.sha256")"
if [[ $actual != "$declared" ]]; then
  echo "SHA-256 distinto: versión='$version', commit='$commit', medido='$actual', declarado='$declared'." >&2
  exit 1
fi
manifest_version="$(sed -n 's/^version=//p' "$staging/build-manifest.txt")"
manifest_commit="$(sed -n 's/^source_commit=//p' "$staging/build-manifest.txt")"
manifest_dirty="$(sed -n 's/^source_dirty=//p' "$staging/build-manifest.txt")"
if [[ $manifest_version != "$version" || $manifest_commit != "$commit" || $manifest_dirty != false ]]; then
  echo "Manifiesto distinto: versión pedida='$version', manifiesto='$manifest_version'; commit pedido='$commit', manifiesto='$manifest_commit'; source_dirty esperado='false', manifiesto='$manifest_dirty'." >&2
  exit 1
fi
# Verify with the verifier and version declaration from the requested commit,
# not this checkout, which may already have advanced to the next release. The
# whole tree of that commit is extracted: which files its verifier reads is the
# verifier's decision, and a fixed list here would be a second, drifting copy
# of it.
mkdir -p -- "$staging/source"
git -C "$repo" archive "$commit" | tar -x -C "$staging/source"
if ! python3 -B "$staging/source/scripts/verify-macos-archive.py" "$staging/$archive" \
  --architecture arm64 --commit "$commit" --checksum-file "$staging/$archive.sha256" \
  --manifest "$staging/build-manifest.txt"; then
  echo "verify-macos-archive.py rechazó versión='$version', commit='$commit'." >&2
  exit 1
fi
if ! ssh "${RELEASE_SSH_OPTIONS[@]}" -i "$ORBIT_MAC_SSH_KEY" "$ORBIT_MAC_HOST" \
  "bash \"\$HOME/orbit-rel-$version/repo/scripts/release/mac-verify.sh\" \"\$HOME/orbit-rel-$version/artifacts/out/$archive\"" \
  > "$staging/verificacion.txt" 2>&1; then
  cat -- "$staging/verificacion.txt" >&2
  echo "Falló la verificación física: versión='$version', commit='$commit'." >&2
  exit 1
fi
rm -rf -- "$staging/source"
mv -- "$staging" "$destination"
trap - EXIT
echo "Verificada versión='$version', commit='$commit': $destination/verificacion.txt"
