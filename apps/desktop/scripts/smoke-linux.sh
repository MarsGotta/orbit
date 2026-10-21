#!/usr/bin/env bash
# The app on Linux, under Xvfb, as one process. It starts its daemon on
# a thread of its own (no `orbit` process), says «daemon ready», and a SIGTERM
# closes the daemon in order and the app with it, exit code 0, socket removed.
set -euo pipefail

if [[ "${1:-}" != "--display-ready" ]]; then
  for _ in 1 2 3; do
    if env \
      __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
      LIBGL_ALWAYS_SOFTWARE=1 \
      xvfb-run -a "$0" --display-ready; then
      exit 0
    fi
  done
  exit 1
fi

SMOKE_DIR="$(mktemp -d /tmp/orbit-app-smoke.XXXXXX)"
APP_PID=""
DESKTOP_BIN="${CARGO_TARGET_DIR:?CARGO_TARGET_DIR is not set}/debug/orbit-desktop"
LOG_FILE="${SMOKE_DIR}/desktop.log"
SOCKET="${SMOKE_DIR}/data/orbit/orbit.sock"
# The app's own waits (apps/desktop/src-tauri/src/daemon_thread.rs): 15 s for
# the daemon's first answer (DAEMON_READY_WAIT) and 17 s for its closure
# (DAEMON_EXIT_WAIT). Each wait here ends on its fact; these only bound a hang.
READY_TENTHS=150
EXIT_TENTHS=170

# Alive and not a zombie: our child stays listed until `wait` reaps it.
alive() {
  local state
  state="$(cut -d' ' -f3 "/proc/$1/stat" 2>/dev/null || true)"
  [[ -n "${state}" && "${state}" != Z ]]
}

# Whether the daemon says its startup recovery is over. Until then a SIGTERM
# ends the start after the step in hand (exit 1, the step named in the log),
# not the closure this smoke is about.
recovered() {
  python3 - "${SOCKET}" <<'PY'
import json, socket, sys
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.settimeout(2)
    connection.connect(sys.argv[1])
    connection.sendall(b'{"v":1,"id":"smoke-linux","body":{"command":"status"}}\n')
    reply = json.loads(connection.makefile("rb").readline() or b"{}")
state = (reply.get("state") or {}).get("recovery") or {}
sys.exit(0 if state.get("state") == "ready" else 1)
PY
}

fail() {
  echo "smoke-linux: $1" >&2
  sed -n '1,160p' "${LOG_FILE}" >&2
  exit 1
}

cleanup() {
  if [[ -n "${APP_PID}" ]] && kill -0 "${APP_PID}" 2>/dev/null; then
    kill -KILL "${APP_PID}" 2>/dev/null || true
    wait "${APP_PID}" 2>/dev/null || true
  fi
  rm -rf -- "${SMOKE_DIR}"
  return 0
}
trap cleanup EXIT

[[ -x "${DESKTOP_BIN}" ]] || { echo "Missing ${DESKTOP_BIN}" >&2; exit 1; }
mkdir -p "${SMOKE_DIR}/runtime"
chmod 700 "${SMOKE_DIR}/runtime"

XDG_DATA_HOME="${SMOKE_DIR}/data" \
XDG_CONFIG_HOME="${SMOKE_DIR}/config" \
XDG_RUNTIME_DIR="${SMOKE_DIR}/runtime" \
GDK_BACKEND=x11 \
WEBKIT_DISABLE_COMPOSITING_MODE=1 \
"${DESKTOP_BIN}" >"${LOG_FILE}" 2>&1 &
APP_PID=$!

ready=0
for _ in $(seq 1 "${READY_TENTHS}"); do
  if grep -q "daemon ready" "${LOG_FILE}"; then
    ready=1
    break
  fi
  alive "${APP_PID}" || fail "the app ended before «daemon ready»"
  sleep 0.1
done
[[ "${ready}" -eq 1 ]] || fail "no «daemon ready» within $((READY_TENTHS / 10)) s"
[[ -S "${SOCKET}" ]] || fail "the socket ${SOCKET} does not exist"

# One process: the daemon is a thread of the app, and no child runs `orbit`.
alive "${APP_PID}" || fail "the app ended right after «daemon ready»"
thread=0
for comm in /proc/"${APP_PID}"/task/*/comm; do
  [[ "$(<"${comm}")" == "orbit-daemon" ]] && thread=1
done
[[ "${thread}" -eq 1 ]] || fail "the app has no orbit-daemon thread"
for children in /proc/"${APP_PID}"/task/*/children; do
  for child in $(<"${children}"); do
    if [[ "$(basename "$(readlink -f "/proc/${child}/exe" 2>/dev/null || true)")" == "orbit" ]]; then
      fail "the app launched an orbit process (PID ${child})"
    fi
  done
done

ready=0
for _ in $(seq 1 "${READY_TENTHS}"); do
  if recovered 2>/dev/null; then
    ready=1
    break
  fi
  alive "${APP_PID}" || fail "the app ended during the startup recovery"
  sleep 0.1
done
[[ "${ready}" -eq 1 ]] || fail "the daemon did not finish its recovery within $((READY_TENTHS / 10)) s"

kill -TERM "${APP_PID}"
exited=0
for _ in $(seq 1 "${EXIT_TENTHS}"); do
  if ! alive "${APP_PID}"; then
    exited=1
    break
  fi
  sleep 0.1
done
[[ "${exited}" -eq 1 ]] || fail "the app was still alive $((EXIT_TENTHS / 10)) s after the SIGTERM"
set +e
wait "${APP_PID}"
status=$?
set -e
APP_PID=""
[[ "${status}" -eq 0 ]] || fail "the app exited with ${status} after the SIGTERM, not 0"
# The app's own log line (src-tauri/src/lib.rs), in the product's Spanish.
grep -q "daemon: el servicio cerró en orden sin que se pidiera salir; Orbit sale" "${LOG_FILE}" \
  || fail "the log does not say the daemon closed in order"
[[ ! -e "${SOCKET}" ]] || fail "the socket ${SOCKET} is still there after the closure"

echo "smoke-linux: daemon ready in a single process and an orderly closure after SIGTERM"
