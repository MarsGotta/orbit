#!/usr/bin/env bash
# Prepare a publishable version folder from the commit that built its package.
set -euo pipefail

release_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$release_dir/../.." && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$release_dir/lib.sh"
release_test_tools
fail() { printf 'prepare-publish: %s\n' "$*" >&2; exit 1; }

version=${1:-}
[[ $# -eq 1 && $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version expected X.Y.Z, found '$version'"
[[ ${ORBIT_RELEASE_DIR:-} = /* ]] || fail 'ORBIT_RELEASE_DIR must be an absolute path'
source_dir="$ORBIT_RELEASE_DIR/$version"
[ -d "$source_dir" ] || fail "version folder missing: $source_dir"
[ -f "$source_dir/build-manifest.txt" ] || fail 'build-manifest.txt missing'
commit=$(sed -n 's/^source_commit=//p' "$source_dir/build-manifest.txt")
release_arguments "$version" "$commit" || fail "invalid source_commit: '$commit'"
git -C "$repo" cat-file -e "$commit^{commit}" 2>/dev/null || fail "no such source_commit: '$commit'"

# The publisher validates all release claims before this command reads the ZIP.
# Staging stays within the version folder and is removed on every refusal.
staging=$(mktemp -d "$source_dir/.prepare.XXXXXXXX")
trap 'rm -rf -- "$staging"' EXIT
# A commit from before scripts/ was reorganised keeps these at scripts/<name>,
# and is prepared from a checkout of its own time.
for path in scripts/release/install.sh scripts/release/publish-macos.sh; do
  git -C "$repo" show "$commit:$path" > "$staging/${path##*/}" || fail "$path missing in source_commit=$commit"
done
printf '%s  install.sh\n' "$(sha256sum "$staging/install.sh" | cut -d' ' -f1)" > "$staging/install.sha256"
# Use the verifier and its whole source tree from the built commit. Its own
# dependencies decide what to inspect; this checkout may have moved ahead.
mkdir "$staging/source"
git -C "$repo" archive "$commit" | tar -x -C "$staging/source"
mkdir "$staging/input"
ln -s "$source_dir/build-manifest.txt" "$staging/input/build-manifest.txt"
for zip in "$source_dir"/Orbit-*-macos-*.zip; do
  [ -e "$zip" ] || continue
  ln -s "$zip" "$staging/input/${zip##*/}"
  if [ -f "$zip.sha256" ]; then
    ln -s "$zip.sha256" "$staging/input/${zip##*/}.sha256"
  fi
done
cp "$staging/install.sh" "$staging/install.sha256" "$staging/input/"
ORBIT_RELEASE_VALIDATE_ONLY=1 bash "$release_dir/publish-macos.sh" \
  "$version" "$staging/input" "$staging/served" || fail 'the publisher refused the version folder'

architecture=$(sed -n 's/^architecture=//p' "$source_dir/build-manifest.txt")
archive="Orbit-$version-macos-$architecture.zip"
python3 -B "$staging/source/scripts/release/verify-macos-archive.py" "$source_dir/$archive" \
  --architecture "$architecture" --commit "$commit" \
  --checksum-file "$source_dir/$archive.sha256" \
  --manifest "$source_dir/build-manifest.txt" || fail "verify-macos-archive.py refused version='$version', commit='$commit'"

# Publish only after the archive and commit have both been checked. Re-running
# preparation restores the same source bytes, including the generated script.
cp "$staging/install.sh" "$source_dir/install.sh"
cp "$staging/install.sha256" "$source_dir/install.sha256"
cp "$staging/publish-macos.sh" "$source_dir/publish-macos.sh"
chmod 755 "$source_dir/publish-macos.sh"
cat > "$source_dir/publish-$version.sh" <<SCRIPT
#!/bin/bash
# Publishes the verified package $version from this folder.
set -euo pipefail
: "\${ORBIT_SERVE_DIR:?ORBIT_SERVE_DIR is missing: the publication destination}"
release_folder="\$(cd -- "\$(dirname -- "\${BASH_SOURCE[0]}")" && pwd)"
exec "\$release_folder/publish-macos.sh" "$version" "\$release_folder" "\$ORBIT_SERVE_DIR"
SCRIPT
chmod 755 "$source_dir/publish-$version.sh"
printf 'Prepared publish-%s.sh: source_commit=%s, installer_sha256=%s\n' \
  "$version" "$commit" "$(cut -d' ' -f1 "$source_dir/install.sha256")"
