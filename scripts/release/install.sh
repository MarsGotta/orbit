#!/bin/bash
# Install Orbit on macOS from a terminal.
#
#   ORBIT_INSTALL_BASE=https://updates.example.test bash install.sh
#
# Verifies the package checksum and installs in /Applications without sudo.
#
# --relaunch starts Orbit; configuration and permissions are separate.
#
# Everything lives inside main() and main runs on the last line, so a download
# cut halfway through executes nothing at all.

set -euo pipefail

: "${ORBIT_INSTALL_BASE:?falta ORBIT_INSTALL_BASE}"
BASE_URL="$ORBIT_INSTALL_BASE"
SYSTEM_APPLICATIONS="/Applications"
INSTALL_DATA_DIRECTORY="Library/Application Support/com.MarsGotta.Orbit"
APP_DIR="${ORBIT_APPLICATIONS_DIR:-$SYSTEM_APPLICATIONS}"
DATA_DIR="$HOME/$INSTALL_DATA_DIRECTORY"

# The scratch directory is global on purpose: the EXIT trap runs after main has
# returned, so a variable local to main would already be gone by then and, under
# `set -u`, a perfectly good install would end in an error at the last step.
WORK=""

# The chain that makes up a running Orbit, one executable each: the app, its
# native helper. Written as the tail of a path because a
# bundle can be installed anywhere, and matched whole so nothing that merely
# lives next to them counts. The engine that travels inside the bundle
# (Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver) is deliberately absent: it is not Orbit, Orbit
# starts it and Orbit stops it.
BUNDLE_NAME="Orbit.app"
CHAIN_APP="/Contents/MacOS/Orbit"
CHAIN_HELPER="/Contents/MacOS/OrbitNative"
PARKED_PREFIX="Orbit.app.anterior-"
INSTALL_RECORD=".orbit-instalado"
LOG_DIR="$HOME/Library/Logs/Orbit"
LOG_FILE="$LOG_DIR/instalador.log"
LOG_ROTATE_BYTES=65536
LOG_KEEP_BYTES=32768
RELAUNCH_REQUESTED=false
KEPT_APP=""
INSTALL_VERSION="desconocida"
INSTALL_NOTE="instalado"
FAIL_REASON=""

# What read_chain found, named in Spanish as the person installing reads it,
# or empty when Orbit is not running.
CHAIN=""

record_line() { printf '%s\n' "$*" >> "$LOG_FILE" || true; }
say() { printf '%s\n' "$*"; record_line "$*"; }
step() { printf '\n\033[1m%s\033[0m\n' "$*"; record_line "$*"; }
fail() {
  FAIL_REASON="$1"
  printf '\n\033[31mNo se completó la instalación.\033[0m %s\n' "$FAIL_REASON" >&2 || true
  exit 1
}

# EXIT restores and reopens even after an implicit set -e exit or a full log.
finish() {
  local status="$1" command="$2" reason code new reopen="$APP_DIR/$BUNDLE_NAME"
  if [ ! -e "$reopen" ] && [ -e "$HOME/Applications/$BUNDLE_NAME" ]; then
    reopen="$HOME/Applications/$BUNDLE_NAME"
  fi
  trap - EXIT
  set +e
  if [ "$status" -ne 0 ]; then
    reason="${FAIL_REASON:-La orden falló con estado $status: $command}"
    if [ -n "$KEPT_APP" ] && [ -e "$KEPT_APP" ]; then
      if [ -e "$APP_DIR/$BUNDLE_NAME" ]; then
        mv "$APP_DIR/$BUNDLE_NAME" "${WORK}/fallida.app" && record_line "App nueva retirada antes de restaurar la anterior"
      fi
      if [ ! -e "$APP_DIR/$BUNDLE_NAME" ]; then
        if mv "$KEPT_APP" "$APP_DIR/$BUNDLE_NAME"; then
          record_line "Versión anterior restaurada"
        else
          code=$?
          new="$WORK/fallida.app"; [ -e "$new" ] || new="$WORK/unpacked/$BUNDLE_NAME"
          reason="$reason; restauración falló (mv: estado $code); anterior: $KEPT_APP; nueva: $new"
          reopen="$KEPT_APP"
          WORK=""
        fi
      fi
    fi
    record_line "FALLO: $reason; estado $status; orden: $command"
    record_line "resultado|$INSTALL_VERSION|$(date -u +%Y-%m-%dT%H:%M:%SZ)|fallo|$reason; estado $status; orden: $command"
    if [ "$RELAUNCH_REQUESTED" = true ] && [ -e "$reopen" ]; then
      if open -a "$reopen"; then
        record_line "Orbit vuelto a abrir tras el fallo"
      else
        record_line "FALLO: no se pudo volver a abrir Orbit"
        record_line "resultado|$INSTALL_VERSION|$(date -u +%Y-%m-%dT%H:%M:%SZ)|fallo|$reason; tampoco se pudo volver a abrir Orbit"
      fi
    fi
  fi
  if [ -n "$WORK" ]; then rm -rf "$WORK" || true; fi
}

# Values arriving over the network are never interpolated unchecked: a manifest
# is data from a server, not a command.
field() {
  local key="$1" file="$2" value
  value=$(grep -m1 "^${key}=" "$file" 2>/dev/null | cut -d= -f2- || true)
  printf '%s' "$value"
}

safe_name() { [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$ ]]; }

safe_digest() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

# Names, in CHAIN, the members of the chain running right now.
#
# A process is judged by the executable it runs, not by a string that appears
# somewhere in its command line. `pgrep -f "Orbit.app/Contents"` did the latter
# and matched anything that so much as mentioned the bundle — that includes
# the engine inside it, which Orbit starts itself, so every update made with
# control enabled waited thirty seconds and then refused. `ps -o comm=` prints the executable's path on macOS, which is
# what a process *is*.
read_chain() {
  local listing
  listing=$(ps -A -ww -o pid=,ppid=,comm= 2>/dev/null || true)
  # `ps` always lists at least this very shell, so nothing at all means the
  # question was not answered. Replacing a bundle without knowing whether its
  # app is running is precisely what this check exists to prevent.
  [ -n "$listing" ] || fail "No se pudo enumerar los procesos con 'ps'; no se instala sin poder comprobar si Orbit sigue abierto."
  CHAIN=$(printf '%s\n' "$listing" | chain_names)
}

# Reads a listing on its input and names what it recognises, in a fixed order so
# the refusal reads the same way every time.
chain_names() {
  local pid ppid path bundle member app="" helper="" found=""
  while read -r pid ppid path; do
    case "$path" in
      *"$CHAIN_APP") bundle=${path%"$CHAIN_APP"}; member=app ;;
      *"$CHAIN_HELPER") bundle=${path%"$CHAIN_HELPER"}; member=helper ;;
      *) continue ;;
    esac
    case "${bundle##*/}" in
      "$BUNDLE_NAME"|"$PARKED_PREFIX"?*) ;;
      *) continue ;;
    esac
    case "$member" in
      app) app="${app:+$app, }la aplicación ($bundle)" ;;
      helper) helper="${helper:+$helper, }su ayudante nativo ($bundle)" ;;
    esac
  done
  if [ -n "$app" ]; then found="$app"; fi
  if [ -n "$helper" ]; then found="${found:+$found, }$helper"; fi
  printf '%s' "$found"
}

main() {
  local relaunch=false
  if [ "${1:-}" = "--relaunch" ]; then
    relaunch=true
    shift
  fi
  RELAUNCH_REQUESTED="$relaunch"
  trap 'finish "$?" "$BASH_COMMAND"' EXIT
  mkdir -p "$LOG_DIR" || true
  # Thirty synthetic installations wrote 695 B each (worst). Reserve 2 KiB
  # per run for a real Mac and keep 32 KiB when the log crosses 64 KiB: at
  # least 16 such runs remain, while the next run fits below the same bound.
  # This bounds local diagnostics, not downloaded content or package storage.
  local log_bytes=0
  if [ -f "$LOG_FILE" ]; then
    log_bytes=$(wc -c < "$LOG_FILE") || log_bytes=0
  fi
  if [ "$log_bytes" -gt "$LOG_ROTATE_BYTES" ]; then
    if tail -c "$LOG_KEEP_BYTES" "$LOG_FILE" > "$LOG_FILE.tmp"; then
      mv "$LOG_FILE.tmp" "$LOG_FILE" || true
    else
      rm -f "$LOG_FILE.tmp" || true
    fi
  fi
  record_line "Inicio $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  [ "$#" -eq 0 ] || fail "Uso: install.sh [--relaunch]"
  if [ "${ORBIT_INSTALLER_VERIFIED:-true}" = false ]; then
    INSTALL_NOTE="instalado; el manifiesto no traía installer_sha256"
    record_line "Aviso: el manifiesto no traía installer_sha256; se ejecuta sin esa verificación"
  fi

  say ""
  say "◉ Orbit"
  say "Companion de macOS. Instalación local, sin sudo."

  [ "$(uname -s)" = "Darwin" ] || fail "Orbit es una app de macOS y esto es $(uname -s)."

  local arch
  case "$(uname -m)" in
    arm64) arch=arm64 ;;
    x86_64) arch=x86_64 ;;
    *) fail "Arquitectura no soportada: $(uname -m)." ;;
  esac

  for tool in curl shasum unzip ps; do
    command -v "$tool" >/dev/null 2>&1 || fail "Falta la herramienta '$tool'."
  done

  # Replacing the bundle under a running process would leave it half-swapped,
  # and a recording in progress is not something to interrupt from a script.
  # The app and its native helper can still be active after the window closes.
  # The refusal names each running member and offers Activity Monitor as well
  # as the Dock and menu bar, so nobody has to guess which process to close.
  if [ "$relaunch" = true ]; then
    # Thirty seconds for the whole chain to leave: the app gives its
    # daemon DAEMON_EXIT_WAIT (17 s) and then exits. More
    # than that, so an update never replaces a bundle whose closure is still
    # within its budget; not a wait anyone normally sits out.
    local wait_deadline=$((SECONDS + 30))
    if [ -n "${ORBIT_INSTALL_READY_SIGNAL:-}" ]; then
      printf '%s\n' "$ORBIT_INSTALL_READY_SIGNAL"
      # The app can exit immediately after the signal. Detach subsequent
      # output from its pipe; diagnostics continue in instalador.log.
      exec >/dev/null
    fi
    read_chain
    while [ -n "$CHAIN" ]; do
      if [ "$SECONDS" -ge "$wait_deadline" ]; then
        fail "Orbit sigue en marcha después de 30 segundos ($CHAIN). Termina esos procesos en Monitor de Actividad y repite este comando."
      fi
      sleep 1
      read_chain
    done
  else
    read_chain
    if [ -n "$CHAIN" ]; then
      fail "Orbit está en marcha ($CHAIN). Ciérralo desde el Dock o su icono de la barra, o termina esos procesos en Monitor de Actividad, y repite este comando."
    fi
  fi

  WORK=$(mktemp -d)

  step "1/4 · Buscando la versión publicada"
  curl -fsSL --max-time 60 "$BASE_URL/latest.txt" -o "${WORK}/latest.txt" \
    || { local code=$?; fail "No se pudo consultar el origen configurado (curl: estado $code). Revisa la conexión y repite la instalación."; }

  local version file digest
  version=$(field version "${WORK}/latest.txt")
  file=$(field "${arch}_file" "${WORK}/latest.txt")
  digest=$(field "${arch}_sha256" "${WORK}/latest.txt")

  safe_name "$version" || fail "El manifiesto no declara una versión utilizable."
  INSTALL_VERSION="$version"
  safe_name "$file" || fail "El manifiesto no declara un paquete utilizable."
  safe_digest "$digest" || fail "El manifiesto no trae un checksum de 64 hex."
  say "  $version · $arch"

  step "2/4 · Descargando y comprobando"
  curl -fSL --max-time 900 --progress-bar "$BASE_URL/packages/$file" -o "${WORK}/$file" \
    || { local code=$?; fail "La descarga no terminó (curl: estado $code)."; }

  local seen
  seen=$(shasum -a 256 "${WORK}/$file" | cut -d' ' -f1) \
    || { local code=$?; fail "No se pudo calcular la huella del paquete (shasum: estado $code)."; }
  if [ "$seen" != "$digest" ]; then
    fail "El paquete $file no coincide con su checksum: esperado $digest; recibido $seen. No se instala."
  fi
  say "  checksum correcto"

  unzip -q "${WORK}/$file" -d "${WORK}/unpacked" \
    || { local code=$?; fail "El paquete no se pudo abrir (unzip: estado $code)."; }
  [ -d "${WORK}/unpacked/$BUNDLE_NAME" ] || fail "El paquete no contiene Orbit.app."

  step "3/4 · Instalando en $APP_DIR"
  mkdir -p "$APP_DIR" \
    || { local code=$?; fail "No se pudo preparar $APP_DIR (mkdir: estado $code). No se instala en otra carpeta."; }
  [ -w "$APP_DIR" ] || fail "No hay permiso de escritura en $APP_DIR (usuario $(id -u)); no se instala en otra carpeta."

  # Keep the previous bundle for rollback. Startup retires the parked copies
  # only after the new daemon has opened the store.
  if [ -e "$APP_DIR/$BUNDLE_NAME" ]; then
    local kept="$APP_DIR/$PARKED_PREFIX$(date +%Y%m%d%H%M%S)"
    mv "$APP_DIR/$BUNDLE_NAME" "$kept" \
      || { local code=$?; fail "No se pudo apartar la versión anterior (mv: estado $code)."; }
    KEPT_APP="$kept"
    say "  versión anterior guardada en $(basename "$kept")"
  elif [ -e "$HOME/Applications/$BUNDLE_NAME" ]; then
    KEPT_APP="$HOME/Applications/$BUNDLE_NAME"
    say "  versión del usuario conservada hasta que la nueva abra el almacén: $KEPT_APP"
  fi
  mv "${WORK}/unpacked/$BUNDLE_NAME" "$APP_DIR/$BUNDLE_NAME" \
    || { local code=$?; fail "No se pudo copiar la app (mv: estado $code)."; }

  local binary="$APP_DIR/$BUNDLE_NAME$CHAIN_APP"
  [ -x "$binary" ] || fail "La app no trae su ejecutable ($CHAIN_APP)."

  # The receipt lives with Orbit's data, outside the signed bundle and
  # outside /Applications. Old user bundles remain the way back until startup.
  mkdir -p "$DATA_DIR" \
    || { local code=$?; fail "No se pudo preparar $DATA_DIR (mkdir: estado $code)."; }
  printf 'version=%s\nsha256=%s\narchitecture=%s\ninstalled=%s\norigin=%s\n' \
    "$version" "$digest" "$arch" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$BASE_URL" \
    > "$DATA_DIR/$INSTALL_RECORD"
  # The receipt now names the new app; a later open failure cannot roll it back.
  KEPT_APP=""

  step "4/4 · Listo"
  say ""
  say "Nada graba todavía y no se ha concedido ningún permiso."
  say "Siguiente paso: abre Orbit; su primer arranque te va preguntando."
  say ""

  # The app reads the last result as soon as open starts it.
  record_line "resultado|$INSTALL_VERSION|$(date -u +%Y-%m-%dT%H:%M:%SZ)|bien|$INSTALL_NOTE"
  if [ "$relaunch" = true ]; then
    RELAUNCH_REQUESTED=false
    open -a "$APP_DIR/$BUNDLE_NAME" \
      || { local code=$?; fail "Orbit se instaló, pero no se pudo volver a abrir (open: estado $code)."; }
  fi
}

main "$@"
