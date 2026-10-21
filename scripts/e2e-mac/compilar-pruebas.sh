#!/usr/bin/env bash
# Builds «Orbit Pruebas» on the Mac from this tree and installs it in
# ~/Applications/Orbit Pruebas.app (SIM-25). Runs on the Mac, by the reviewer:
#
#   ORBIT_CUA_DRIVER_BINARY=/ruta/cua-driver ORBIT_SIGNING_ENV=/ruta/orbit-signing.env \
#     bash scripts/e2e-mac/compilar-pruebas.sh
#
# The same packaging as a release (scripts/bundle-macos.sh) with ORBIT_PRUEBAS=1,
# signed with the same stable identity — so the permissions Mars grants Orbit
# Pruebas once survive the next build — and never notarized: it is not
# published. Only the three signing keys are taken from the signing file; the
# notarization ones are left out on purpose. Nothing here touches Orbit.app.
set -euo pipefail

repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
# shellcheck source=scripts/release/lib.sh
source "$repo/scripts/release/lib.sh"
# A non-interactive SSH session needs the Mac's build tools on its PATH.
export PATH="${RELEASE_HOST_PATH//\$HOME/$HOME}"

destino="$HOME/Applications/Orbit Pruebas.app"

if [[ $(uname -s) != Darwin ]]; then
  echo "Orbit Pruebas se compila en el Mac." >&2
  exit 1
fi
if [[ ! ${ORBIT_SIGNING_ENV:-} = /* || ! -f ${ORBIT_SIGNING_ENV:-} || -L ${ORBIT_SIGNING_ENV:-} ]]; then
  echo "ORBIT_SIGNING_ENV debe ser la ruta absoluta del fichero de firma: '${ORBIT_SIGNING_ENV:-}'." >&2
  exit 1
fi
release_read_signing_env
for key in ORBIT_SIGN_IDENTITY ORBIT_SIGN_KEYCHAIN ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE; do
  if [[ -z ${!key:-} ]]; then
    echo "Falta $key en $ORBIT_SIGNING_ENV." >&2
    exit 1
  fi
done
unset ORBIT_NOTARY_KEY ORBIT_NOTARY_KEY_ID ORBIT_NOTARY_ISSUER

salida="$(mktemp -d)"
trap 'rm -rf -- "$salida"' EXIT
ORBIT_PRUEBAS=1 ORBIT_OUTPUT_DIR="$salida/out" bash "$repo/scripts/bundle-macos.sh"

identificador="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$salida/out/Orbit Pruebas.app/Contents/Info.plist")"
if [[ $identificador != app.orbit.companion.pruebas ]]; then
  echo "El paquete construido es «$identificador», no Orbit Pruebas: no se instala." >&2
  exit 1
fi
if ps -axo comm= | grep -qF -- "$destino/Contents/"; then
  echo "Orbit Pruebas está abierta; ciérrala antes de instalar la nueva." >&2
  exit 1
fi
# Only this exact path, which is Orbit Pruebas's own.
rm -rf -- "$destino"
mkdir -p -- "$HOME/Applications"
ditto "$salida/out/Orbit Pruebas.app" "$destino"
codesign --verify --deep --strict "$destino"
echo "Instalada: $destino ($identificador)"
