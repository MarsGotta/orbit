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
  echo "Versión distinta en Cargo.toml: pedida='$version', declarada='$declared', commit='$commit'." >&2
  exit 1
fi

staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT
git clone --quiet --no-hardlinks --no-checkout "$repo" "$staging/repo"
git -C "$staging/repo" checkout --quiet --detach "$commit"
if ! check_output="$(cd "$staging/repo" && python3 -B scripts/set-version.py --check 2>&1)"; then
  echo "set-version.py --check rechazó versión='$version', commit='$commit': $check_output" >&2
  exit 1
fi
release_connection
if [[ ! ${ORBIT_CUA_DRIVER_BINARY:-} =~ ^/[A-Za-z0-9_./-]+$ ]]; then
  echo 'Falta ORBIT_CUA_DRIVER_BINARY: ruta absoluta del driver en el Mac.' >&2
  exit 2
fi
if [[ ! ${ORBIT_SIGNING_ENV:-} =~ ^/[A-Za-z0-9_./-]+$ ]]; then
  echo 'Falta ORBIT_SIGNING_ENV: ruta absoluta del fichero privado en el Mac.' >&2
  exit 2
fi
# SIM-23: the Sentry address official builds carry. It never lives in the
# repository: it is read from ORBIT_SENTRY_DSN_FILE (by default
# ~/.orbit/sentry/orbit-app.dsn on this machine) and reaches the Mac as a
# private file beside the build, never on a command line. An empty
# ORBIT_SENTRY_DSN_FILE builds without it: no switch, and nothing can leave.
sentry_file=${ORBIT_SENTRY_DSN_FILE-$HOME/.orbit/sentry/orbit-app.dsn}
if [[ -n $sentry_file && ( ! -f $sentry_file || -L $sentry_file ) ]]; then
  echo "No existe el fichero de la dirección de Sentry: ORBIT_SENTRY_DSN_FILE='$sentry_file'. Déjalo vacío para compilar sin informes de errores." >&2
  exit 2
fi
remote="\$HOME/orbit-rel-$version"
if ! ssh "${RELEASE_SSH_OPTIONS[@]}" -i "$ORBIT_MAC_SSH_KEY" "$ORBIT_MAC_HOST" "test ! -e \"$remote\" && mkdir -- \"$remote\""; then
  echo "La carpeta de compilación ya existe o no se pudo crear en el Mac: versión='$version', commit='$commit', carpeta='~/orbit-rel-$version'." >&2
  exit 1
fi
# The clone contains only tracked files at this commit and its .git directory.
# There are no excludes: even .git/refs/heads/main must reach the Mac.
if ! rsync -az --delete -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" "$staging/repo/" "$ORBIT_MAC_HOST:~/orbit-rel-$version/repo/"; then
  echo "No se pudo enviar el commit al Mac: versión='$version', commit='$commit'." >&2
  exit 1
fi
remote_sentry=
if [[ -n $sentry_file ]]; then
  if ! rsync -a --chmod=F600 -e "ssh ${RELEASE_SSH_OPTIONS[*]} -i $ORBIT_MAC_SSH_KEY" "$sentry_file" "$ORBIT_MAC_HOST:~/orbit-rel-$version/sentry.dsn"; then
    echo "No se pudo enviar la dirección de Sentry al Mac: versión='$version', commit='$commit'." >&2
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
  echo "No se pudo lanzar la compilación en el Mac: versión='$version', commit='$commit'." >&2
  exit 1
fi
echo "Compilación lanzada: versión='$version', commit='$commit'; registro en $ORBIT_MAC_HOST:~/orbit-rel-$version/build.log"
