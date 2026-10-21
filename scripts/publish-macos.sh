#!/bin/bash
# Publish a verified Orbit build from its version folder.
#
#   scripts/publish-macos.sh <version> <version-dir> <serve-dir>
#
# The version folder holds what was built: the ZIP, its .sha256, the
# build-manifest.txt, and install.sh with install.sha256 taken from the built
# commit by scripts/release/prepare-publish.sh. The installer served is that
# one, never this checkout's: it is the installer that was built and verified
# with that package, and main may have moved on since (REF-503b A2).
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
  [ "$count" = 1 ] || fail "build-manifest.txt campo $key esperado una vez, encontrado $count"
  sed -n "s/^${key}=//p" "$file"
}
require_field() {
  local key="$1" expected="$2" found
  found=$(field "$key" "$source_dir/build-manifest.txt")
  [ "$found" = "$expected" ] || fail "build-manifest.txt campo $key esperado='$expected', encontrado='$found'"
}
sha() { sha256sum "$1" | cut -d' ' -f1; }

[ $# -eq 3 ] || fail 'uso: publish-macos.sh <version> <dir-versión> <serve-dir>'
version="$1"
source_dir="$2"
serve_dir="$3"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "versión no válida: '$version'"
[ -d "$source_dir" ] || fail "no existe $source_dir"
# A test fixture has no authority to publish to the live origin, even when a
# generated command accidentally receives a production destination.
if [[ ${ORBIT_RELEASE_UNDER_TEST:-} == 1 && ( $serve_dir == /srv || $serve_dir == /srv/* ) ]]; then
  fail "destino $serve_dir bajo /srv prohibido en tests"
fi
[ -f "$source_dir/build-manifest.txt" ] || fail "build-manifest.txt esperado para $version, encontrado ausente"
require_field version "$version"
commit=$(field source_commit "$source_dir/build-manifest.txt")
[[ $commit =~ ^[0-9a-f]{40}$ ]] || fail "build-manifest.txt campo source_commit esperado=40 hex minúsculos, encontrado='$commit'"
require_field source_dirty false
require_field notarized true

shopt -s nullglob
all_zips=("$source_dir"/Orbit-*.zip "$source_dir"/*/Orbit-*.zip)
[ ${#all_zips[@]} -gt 0 ] || fail "paquete Orbit-$version-macos-<arq>.zip esperado, encontrado ausente"
for zip in "${all_zips[@]}"; do
  name=${zip##*/}
  case "$zip" in
    "$source_dir/Orbit-$version-macos-arm64.zip"|"$source_dir/Orbit-$version-macos-x86_64.zip") ;;
    *) fail "paquete $name encontrado; esperado Orbit-$version-macos-<arq>.zip" ;;
  esac
done

arches=()
for arch in arm64 x86_64; do
  zip="$source_dir/Orbit-$version-macos-$arch.zip"
  [ -f "$zip" ] && arches+=("$arch")
done
[ ${#arches[@]} -gt 0 ] || fail "paquete Orbit-$version-macos-<arq>.zip esperado, encontrado ausente"
[ ${#arches[@]} -eq 1 ] || fail "build-manifest.txt campo architecture esperado=una arquitectura, encontrados=${arches[*]}"
require_field architecture "${arches[0]}"

installer="$source_dir/install.sh"
[ -f "$installer" ] || fail "install.sh esperado para source_commit=$commit, encontrado ausente"
[ -f "$source_dir/install.sha256" ] || fail "install.sha256 esperado para source_commit=$commit, encontrado ausente"
expected_installer=$(cut -d' ' -f1 "$source_dir/install.sha256")
[[ $expected_installer =~ ^[0-9a-f]{64}$ ]] || fail "install.sha256 esperado=64 hex minúsculos, encontrado='$expected_installer'"
actual_installer=$(sha "$installer")
[ "$actual_installer" = "$expected_installer" ] || fail "install.sh source_commit=$commit sha256 esperado='$expected_installer', encontrado='$actual_installer'"
# The updater of every Mac already installed downloads at most 16,756 B
# (INSTALLER_CEILING_BYTES in crates/orbit-update/src/lib.rs: 11,171 B
# baseline plus 50% headroom). A larger installer could not reach them. It
# does not authenticate or sandbox shell code.
INSTALLER_CEILING_BYTES=16756
installer_bytes=$(wc -c < "$installer")
[ "$installer_bytes" -le "$INSTALLER_CEILING_BYTES" ] || fail "install.sh mide $installer_bytes B; techo del actualizador=$INSTALLER_CEILING_BYTES B"

# Validate all inputs before touching the served directory.
for arch in "${arches[@]}"; do
  zip="$source_dir/Orbit-$version-macos-$arch.zip"
  [ -f "$zip.sha256" ] || fail "$(basename "$zip").sha256 esperado, encontrado ausente"
  claimed=$(cut -d' ' -f1 "$zip.sha256")
  measured=$(sha "$zip")
  [ "$claimed" = "$measured" ] || fail "$(basename "$zip") sha256 esperado='$claimed', encontrado='$measured'"
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
      /bin/mv "$serve_dir/install.sh.anterior" "$serve_dir/install.sh" || printf 'publish: no se pudo restaurar install.sh\n' >&2
    else
      rm -f "$serve_dir/install.sh" || printf 'publish: no se pudo retirar install.sh\n' >&2
    fi
  fi
  rm -f "$manifest" "$serve_dir/install.sh.parcial" "$serve_dir/latest.txt.parcial" "$serve_dir/install.sh.anterior"
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
    [ "$current" = "$measured" ] || fail "$name ya está publicado con otro contenido: esperado='$measured', encontrado='$current'; sube la versión"
  else
    cp "$zip" "$target.parcial"
    chmod 644 "$target.parcial"
    mv "$target.parcial" "$target"
  fi
  published_digest=$(sha "$target")
  [ "$published_digest" = "$measured" ] || fail "$name sha256 de origen='$measured', servido='$published_digest'"
  printf '%s_file=%s\n%s_sha256=%s\n' "$arch" "$name" "$arch" "$published_digest" >> "$manifest"
done
printf 'source_commit=%s\ninstaller_sha256=%s\n' "$commit" "$actual_installer" >> "$manifest"

# Install first: a new installer can read the old manifest. A new manifest
# before its installer would advertise a digest that is not served yet.
# On a failed second rename, restore the previous installer. Each rename is
# atomic; the two-file pair still needs an external publication lock to avoid
# a reader observing the interval between renames.
if [ -f "$serve_dir/install.sh" ]; then
  cp -p "$serve_dir/install.sh" "$serve_dir/install.sh.anterior"
  chmod 644 "$serve_dir/install.sh.anterior"
  had_installer=1
fi
cp "$installer" "$serve_dir/install.sh.parcial"
chmod 644 "$serve_dir/install.sh.parcial"
mv "$serve_dir/install.sh.parcial" "$serve_dir/install.sh"
installer_changed=1
chmod 644 "$serve_dir/install.sh"
served_installer_sha=$(sha "$serve_dir/install.sh")
[ "$served_installer_sha" = "$actual_installer" ] || fail "install.sh sha256 preparado='$actual_installer', servido='$served_installer_sha'"
cp "$manifest" "$serve_dir/latest.txt.parcial"
chmod 644 "$serve_dir/latest.txt.parcial"
mv "$serve_dir/latest.txt.parcial" "$serve_dir/latest.txt"
publish_committed=1
chmod 644 "$serve_dir/latest.txt" "$packages"/*.zip
printf 'publish: manifiesto listo con %s paquete(s), source_commit=%s, installer_sha256=%s\n' "${#arches[@]}" "$commit" "$actual_installer"
