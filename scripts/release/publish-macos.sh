#!/bin/bash
# Publish a verified Orbit build from its version folder.
#
#   scripts/release/publish-macos.sh <version> <version-dir> <serve-dir>
#
# The version folder holds what was built: the ZIP, its .sha256, the
# build-manifest.txt, and install.sh with install.sha256 taken from the built
# commit by scripts/release/prepare-publish.sh. The installer served is that
# one, never this checkout's: it is the installer that was built and verified
# with that package, and main may have moved on since.
#
# The digests written to the manifest are MEASURED here from the files being
# published. The .sha256 files that travel with a build are checked too, but
# they are treated as a claim to verify, not as the answer to copy: a manifest
# that repeats an unverified digest would certify nothing.
#
# Nothing is deleted. A version already published is refused rather than
# overwritten, so a package someone already installed cannot change underneath
# them and still answer to the same name.
#
# Every file reaches the serve dir readable by all (0644) before its rename:
# the web server reads as another user, and the shell's umask here is 007.
set -euo pipefail

fail() { printf 'publish: %s\n' "$*" >&2; exit 1; }
field() {
  local key="$1" file="$2" count
  count=$(grep -c "^${key}=" "$file" || true)
  [ "$count" = 1 ] || fail "build-manifest.txt field $key expected once, found $count"
  sed -n "s/^${key}=//p" "$file"
}
require_field() {
  local key="$1" expected="$2" found
  found=$(field "$key" "$source_dir/build-manifest.txt")
  [ "$found" = "$expected" ] || fail "build-manifest.txt field $key expected='$expected', found='$found'"
}
sha() { sha256sum "$1" | cut -d' ' -f1; }

[ $# -eq 3 ] || fail 'usage: publish-macos.sh <version> <version-dir> <serve-dir>'
version="$1"
source_dir="$2"
serve_dir="$3"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "invalid version: '$version'"
[ -d "$source_dir" ] || fail "$source_dir does not exist"
# A test fixture has no authority to publish to the live origin, even when a
# generated command accidentally receives a production destination.
if [[ ${ORBIT_RELEASE_UNDER_TEST:-} == 1 && ( $serve_dir == /srv || $serve_dir == /srv/* ) ]]; then
  fail "destination $serve_dir under /srv is forbidden in tests"
fi
[ -f "$source_dir/build-manifest.txt" ] || fail "build-manifest.txt expected for $version, found absent"
require_field version "$version"
commit=$(field source_commit "$source_dir/build-manifest.txt")
[[ $commit =~ ^[0-9a-f]{40}$ ]] || fail "build-manifest.txt field source_commit expected=40 lowercase hex, found='$commit'"
require_field source_dirty false
require_field notarized true

shopt -s nullglob
all_zips=("$source_dir"/Orbit-*.zip "$source_dir"/*/Orbit-*.zip)
[ ${#all_zips[@]} -gt 0 ] || fail "package Orbit-$version-macos-<arch>.zip expected, found absent"
for zip in "${all_zips[@]}"; do
  name=${zip##*/}
  case "$zip" in
    "$source_dir/Orbit-$version-macos-arm64.zip"|"$source_dir/Orbit-$version-macos-x86_64.zip") ;;
    *) fail "package $name found; expected Orbit-$version-macos-<arch>.zip" ;;
  esac
done

arches=()
for arch in arm64 x86_64; do
  zip="$source_dir/Orbit-$version-macos-$arch.zip"
  [ -f "$zip" ] && arches+=("$arch")
done
[ ${#arches[@]} -gt 0 ] || fail "package Orbit-$version-macos-<arch>.zip expected, found absent"
[ ${#arches[@]} -eq 1 ] || fail "build-manifest.txt field architecture expected=one architecture, found=${arches[*]}"
require_field architecture "${arches[0]}"

installer="$source_dir/install.sh"
[ -f "$installer" ] || fail "install.sh expected for source_commit=$commit, found absent"
[ -f "$source_dir/install.sha256" ] || fail "install.sha256 expected for source_commit=$commit, found absent"
expected_installer=$(cut -d' ' -f1 "$source_dir/install.sha256")
[[ $expected_installer =~ ^[0-9a-f]{64}$ ]] || fail "install.sha256 expected=64 lowercase hex, found='$expected_installer'"
actual_installer=$(sha "$installer")
[ "$actual_installer" = "$expected_installer" ] || fail "install.sh source_commit=$commit sha256 expected='$expected_installer', found='$actual_installer'"
# The updater of every Mac already installed downloads at most 16,756 B
# (INSTALLER_CEILING_BYTES in crates/orbit-app-core/src/update/download.rs: 11,171 B
# baseline plus 50% headroom). A larger installer could not reach them. It
# does not authenticate or sandbox shell code.
INSTALLER_CEILING_BYTES=16756
installer_bytes=$(wc -c < "$installer")
[ "$installer_bytes" -le "$INSTALLER_CEILING_BYTES" ] || fail "install.sh measures $installer_bytes B; updater ceiling=$INSTALLER_CEILING_BYTES B"

# Validate all inputs before touching the served directory.
for arch in "${arches[@]}"; do
  zip="$source_dir/Orbit-$version-macos-$arch.zip"
  [ -f "$zip.sha256" ] || fail "$(basename "$zip").sha256 expected, found absent"
  claimed=$(cut -d' ' -f1 "$zip.sha256")
  measured=$(sha "$zip")
  [ "$claimed" = "$measured" ] || fail "$(basename "$zip") sha256 expected='$claimed', found='$measured'"
done

[[ ${ORBIT_RELEASE_VALIDATE_ONLY:-} == 1 ]] && exit 0

packages="$serve_dir/packages"
mkdir -p "$packages"
manifest=$(mktemp)
had_installer=0
installer_changed=0
publish_committed=0
cleanup() {
  local status=$?
  if [ "$installer_changed" -eq 1 ] && [ "$publish_committed" -eq 0 ]; then
    if [ "$had_installer" -eq 1 ]; then
      /bin/mv "$serve_dir/install.sh.previous" "$serve_dir/install.sh" || printf 'publish: install.sh could not be restored\n' >&2
    else
      rm -f "$serve_dir/install.sh" || printf 'publish: install.sh could not be removed\n' >&2
    fi
  fi
  rm -f "$manifest" "$serve_dir/install.sh.partial" "$serve_dir/latest.txt.partial" "$serve_dir/install.sh.previous"
  return "$status"
}
trap cleanup EXIT
printf 'version=%s\n' "$version" >> "$manifest"
printf 'published=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$manifest"
for arch in "${arches[@]}"; do
  zip="$source_dir/Orbit-$version-macos-$arch.zip"
  name=${zip##*/}
  target="$packages/$name"
  measured=$(sha "$zip")
  if [ -e "$target" ]; then
    current=$(sha "$target")
    [ "$current" = "$measured" ] || fail "$name is already published with other content: expected='$measured', found='$current'; raise the version"
  else
    cp "$zip" "$target.partial"
    chmod 644 "$target.partial"
    mv "$target.partial" "$target"
  fi
  published_digest=$(sha "$target")
  [ "$published_digest" = "$measured" ] || fail "$name sha256 at the source='$measured', served='$published_digest'"
  printf '%s_file=%s\n%s_sha256=%s\n' "$arch" "$name" "$arch" "$published_digest" >> "$manifest"
done
printf 'source_commit=%s\ninstaller_sha256=%s\n' "$commit" "$actual_installer" >> "$manifest"

# Install first: a new installer can read the old manifest. A new manifest
# before its installer would advertise a digest that is not served yet.
# On a failed second rename, restore the previous installer. Each rename is
# atomic; the two-file pair still needs an external publication lock to avoid
# a reader observing the interval between renames.
if [ -f "$serve_dir/install.sh" ]; then
  cp -p "$serve_dir/install.sh" "$serve_dir/install.sh.previous"
  chmod 644 "$serve_dir/install.sh.previous"
  had_installer=1
fi
cp "$installer" "$serve_dir/install.sh.partial"
chmod 644 "$serve_dir/install.sh.partial"
mv "$serve_dir/install.sh.partial" "$serve_dir/install.sh"
installer_changed=1
chmod 644 "$serve_dir/install.sh"
served_installer_sha=$(sha "$serve_dir/install.sh")
[ "$served_installer_sha" = "$actual_installer" ] || fail "install.sh sha256 prepared='$actual_installer', served='$served_installer_sha'"
cp "$manifest" "$serve_dir/latest.txt.partial"
chmod 644 "$serve_dir/latest.txt.partial"
mv "$serve_dir/latest.txt.partial" "$serve_dir/latest.txt"
publish_committed=1
chmod 644 "$serve_dir/latest.txt" "$packages"/*.zip
printf 'publish: manifest ready with %s package(s), source_commit=%s, installer_sha256=%s\n' "${#arches[@]}" "$commit" "$actual_installer"
