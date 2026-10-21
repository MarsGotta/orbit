#!/usr/bin/env bash
# Orbit's gate: one command that checks everything a Linux machine can check.
set -euo pipefail

if ((BASH_VERSINFO[0] < 5)); then
  printf 'check.sh needs bash 5 or later and this is %s (on macOS: brew install bash)\n' \
    "$BASH_VERSION" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK_DIR="$ROOT/scripts/check"
# shellcheck source=scripts/check/runner.sh
source "$CHECK_DIR/runner.sh"

DEFAULT_ZONES=(rust guards hub ui)
KNOWN_ZONES=(rust guards hub ui)
GENERATED_TYPES="$ROOT/apps/desktop/src/bridge/generated"
# The daemon as a process of its own, target/debug/orbit: what the hub's e2e,
# app-core's tests and the daemon's process tests start. It is not shipped:
# the published daemon is compiled inside orbit-desktop.
DAEMON_PACKAGE=orbit-daemon
DAEMON_BUILD=(cargo build -p "$DAEMON_PACKAGE" --bin orbit --locked)
# The Tauri app, tested in a `cargo test` of its own. Cargo unifies the
# features of every package one command compiles: next to the app, the daemon
# gets the features Tauri asks of their shared dependencies (15 crates on Linux
# and 8 on macOS, measured when the split was made). That is the daemon that
# ships, inside the app; the daemon's own suite runs without them, and the
# app's suite runs the daemon in-process with them.
APP_PACKAGES=(orbit-app-core orbit-desktop)
# The step that last left the published daemon in target/debug/orbit: whatever
# starts that file needs it (see published_daemon_again).
DAEMON_STEP=daemon
# The dry run of the Mac's real-test harness: a pytest module that needs the
# hub's environment, so the hub zone runs it, not the guards' unittest.
HARNESS_TEST=../scripts/tests/mac_harness_dry_run_test.py

usage() {
  cat <<'EOF'
Usage: scripts/check.sh [options] [zone...]

With no zone it runs the whole gate: rust, guards, hub and ui, in that order.
Every step appears in a final table (step, result, duration) and the exit code
is non-zero if any of them fails.

Zones:
  rust      format, strict clippy, cargo test --workspace and generated TypeScript
  guards    python3 -B -m unittest discover -s scripts/tests (after the hub's
            environment: the hub publish guards run uv offline from its cache)
  hub       ruff, format, the hub's pytest with its e2e, and the harness dry run
  ui        lint, typecheck, vitest and build of apps/desktop

Options:
  --plan       lists the steps, in order, without running anything
  --install    creates or syncs server/.venv (uv) and apps/desktop/node_modules (pnpm)
  -h, --help   this help

Variables:
  CARGO_TARGET_DIR             where Cargo builds (by default, the repository's target/)
  ORBIT_CHECK_LOG_DIR          where each step's output is kept
EOF
}

CHECK_PLAN=""
INSTALL=""
requested=()
for argument in "$@"; do
  case "$argument" in
    --plan) CHECK_PLAN=1 ;;
    --install) INSTALL=1 ;;
    -h | --help) usage; exit 0 ;;
    -*) printf 'unknown option: %s\n\n' "$argument" >&2; usage >&2; exit 2 ;;
    *)
      if [[ " ${KNOWN_ZONES[*]} " != *" $argument "* ]]; then
        printf "unknown zone '%s': the zones are %s\n" "$argument" "${KNOWN_ZONES[*]}" >&2
        exit 2
      fi
      requested+=("$argument")
      ;;
  esac
done
((${#requested[@]})) || requested=("${DEFAULT_ZONES[@]}")
declare -A ZONE=()
for zone in "${requested[@]}"; do ZONE["$zone"]=1; done
selected() {
  local zone
  for zone in "$@"; do [[ -n "${ZONE[$zone]:-}" ]] && return 0; done
  return 1
}

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
[[ "$TARGET_DIR" == /* ]] || TARGET_DIR="$PWD/$TARGET_DIR"
export CARGO_TARGET_DIR="$TARGET_DIR"
DAEMON="$TARGET_DIR/debug/orbit"
LOG_DIR="${ORBIT_CHECK_LOG_DIR:-$TARGET_DIR/check/$(date +%Y%m%d-%H%M%S)}"
# Colours off for every tool; skips.py strips whatever colour still arrives
# before it reads a single line. No FORCE_COLOR: Node warns on every run
# when it is set next to NO_COLOR.
export NO_COLOR=1 CARGO_TERM_COLOR=never PY_COLORS=0 CLICOLOR=0

if [[ -z "$CHECK_PLAN" ]]; then
  if ! command -v python3 >/dev/null || ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    printf 'check.sh needs python3 3.11 or later on the PATH (it reads TOML and the report of each suite)\n' >&2
    exit 2
  fi
fi

# ── Environments ────────────────────────────────────────────────────────────

# First `key = "value"` of a TOML-like file, without a pipe that could fail.
first_string_value() {
  local file="$1" key="$2" line
  [[ -f "$file" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ "$line" =~ ^[[:space:]]*$key[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
      printf '%s' "${BASH_REMATCH[1]}"
      return 0
    fi
  done <"$file"
}

RUST_CHANNEL="$(first_string_value "$ROOT/rust-toolchain.toml" channel)"
RUST_ENV_STATUS=1
RUST_ENV_MESSAGE=""

host_triple() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) printf 'x86_64-unknown-linux-gnu' ;;
    Linux-aarch64 | Linux-arm64) printf 'aarch64-unknown-linux-gnu' ;;
    Darwin-arm64) printf 'aarch64-apple-darwin' ;;
    Darwin-x86_64) printf 'x86_64-apple-darwin' ;;
    *) return 1 ;;
  esac
}

# Finds the toolchain rust-toolchain.toml pins, without installing anything, and
# puts it first on the PATH of this shell so every later step uses it.
discover_rust_toolchain() {
  if [[ -z "$RUST_CHANNEL" ]]; then
    RUST_ENV_MESSAGE="rust-toolchain.toml pins no channel in $ROOT/rust-toolchain.toml"
    return
  fi
  local triple directory missing=() tool found_version=""
  if ! triple="$(host_triple)"; then
    RUST_ENV_MESSAGE="no known toolchain for $(uname -s) $(uname -m)"
    return
  fi
  directory="${RUSTUP_HOME:-$HOME/.rustup}/toolchains/$RUST_CHANNEL-$triple/bin"
  for tool in cargo cargo-clippy rustfmt; do
    [[ -x "$directory/$tool" ]] || missing+=("$tool")
  done
  if ((${#missing[@]} == 0)); then
    export PATH="$directory:$PATH"
    RUST_ENV_STATUS=0
    RUST_ENV_MESSAGE="toolchain $RUST_CHANNEL-$triple in $directory"
    return
  fi
  if command -v cargo >/dev/null; then
    found_version="$(RUSTUP_AUTO_INSTALL=0 cargo --version 2>/dev/null || true)"
    if [[ "$found_version" == "cargo $RUST_CHANNEL "* ]] \
      && RUSTUP_AUTO_INSTALL=0 cargo clippy --version >/dev/null 2>&1 \
      && RUSTUP_AUTO_INSTALL=0 cargo fmt --version >/dev/null 2>&1; then
      RUST_ENV_STATUS=0
      RUST_ENV_MESSAGE="$found_version from the PATH ($(command -v cargo)), with clippy and rustfmt"
      return
    fi
  fi
  RUST_ENV_MESSAGE="the Rust toolchain $RUST_CHANNEL ($triple) that rust-toolchain.toml pins is missing. \
Missing in $directory: ${missing[*]}. On the PATH, cargo is: ${found_version:-none}. \
Install it with: rustup toolchain install $RUST_CHANNEL --profile minimal --component clippy --component rustfmt"
}

report_rust_environment() {
  printf '%s\n' "$RUST_ENV_MESSAGE"
  return "$RUST_ENV_STATUS"
}

HUB_SYNC=(uv sync --frozen --group dev)

check_hub_environment() {
  local venv="$ROOT/server/.venv" resolved expected tool
  if [[ -n "$INSTALL" ]]; then
    command -v uv >/dev/null || {
      printf 'uv is missing from the PATH to create server/.venv (https://docs.astral.sh/uv/)\n'
      return 1
    }
    (cd "$ROOT/server" && "${HUB_SYNC[@]}") || return
  elif [[ ! -x "$venv/bin/python" ]]; then
    printf "server/.venv, the hub's environment, is missing. Create it with: (cd server && %s), or run scripts/check.sh --install\n" \
      "${HUB_SYNC[*]}"
    return 1
  fi
  for tool in python ruff pytest; do
    [[ -x "$venv/bin/$tool" ]] || {
      printf 'server/.venv has no %s: sync it with (cd server && %s)\n' "$tool" "${HUB_SYNC[*]}"
      return 1
    }
  done
  # From /, so the working directory cannot supply orbit_hub: a venv copied from
  # another checkout resolves it there and ~29 tests fail for no reason.
  resolved="$(cd / && "$venv/bin/python" -c \
    'import orbit_hub, pathlib; print(pathlib.Path(orbit_hub.__file__).resolve().parents[1])')" || {
    printf 'server/.venv cannot import orbit_hub; sync it with (cd server && %s)\n' "${HUB_SYNC[*]}"
    return 1
  }
  expected="$(cd "$ROOT/server" && pwd -P)"
  if [[ "$resolved" != "$expected" ]]; then
    printf 'server/.venv imports orbit_hub from %s, not from this tree (%s): recreate it here with (cd server && %s)\n' \
      "$resolved" "$expected" "${HUB_SYNC[*]}"
    return 1
  fi
  printf 'server/.venv: %s, orbit_hub from %s\n' "$("$venv/bin/python" --version 2>&1)" "$resolved"
}

check_ui_environment() {
  local desktop="$ROOT/apps/desktop" tool
  for tool in node pnpm; do
    command -v "$tool" >/dev/null || {
      case "$tool" in
        pnpm) printf 'pnpm is missing from the PATH (package.json pins pnpm 9.15.4; with Node: corepack enable)\n' ;;
        *) printf '%s is missing from the PATH\n' "$tool" ;;
      esac
      return 1
    }
  done
  if [[ -n "$INSTALL" ]]; then
    (cd "$desktop" && pnpm install --frozen-lockfile) || return
  elif [[ ! -d "$desktop/node_modules" ]]; then
    printf "the interface's dependencies are missing: (cd apps/desktop && pnpm install --frozen-lockfile), or run scripts/check.sh --install\n"
    return 1
  elif ! cmp -s "$desktop/pnpm-lock.yaml" "$desktop/node_modules/.pnpm/lock.yaml"; then
    printf 'apps/desktop/node_modules does not match pnpm-lock.yaml: (cd apps/desktop && pnpm install --frozen-lockfile), or run scripts/check.sh --install\n'
    return 1
  fi
  printf 'node %s, pnpm %s, dependencies as pnpm-lock.yaml says\n' "$(node --version)" "$(pnpm --version)"
}

# ── Steps ───────────────────────────────────────────────────────────────────

hub_pytest() {
  local id="$1" label="$2"
  shift 2
  check_run "$id" "$label" --needs env-hub --needs "$DAEMON_STEP" --cwd "$ROOT/server" \
    --report "junit:$LOG_DIR/$id.xml" -- \
    env "ORBIT_TEST_BINARY=$DAEMON" .venv/bin/python -m pytest -q "--junitxml=$LOG_DIR/$id.xml" "$@"
}

# `cargo test` of orbit-daemon links `orbit` again with the features of its tests
# (tokio/test-util, a dev-dependency) and leaves it in target/debug/orbit: the
# daemon that the app tests and the hub e2e start. This puts the one without
# test features back; when nothing changed it only links the file already
# compiled (0.2 s).
published_daemon_again() {
  local id="$1" zone="$2" after="$3"
  check_run "$id" "$zone · target/debug/orbit is the daemon without test features again" --after "$after" \
    --cwd "$ROOT" -- "${DAEMON_BUILD[@]}"
  DAEMON_STEP="$id"
}

run_rust_zone() {
  check_run rust-fmt "rust · format" --needs env-rust --cwd "$ROOT" -- \
    cargo fmt --all -- --check
  check_run rust-clippy "rust · strict clippy" --needs env-rust --cwd "$ROOT" -- \
    cargo clippy --workspace --all-targets --locked -- -D warnings
  if [[ -z "$CHECK_PLAN" ]]; then
    python3 -B "$CHECK_DIR/generated.py" snapshot "$GENERATED_TYPES" "$LOG_DIR/generated.json" || true
  fi
  local package without_app=() app=()
  for package in "${APP_PACKAGES[@]}"; do
    without_app+=(--exclude "$package")
    app+=(-p "$package")
  done
  # Two commands, so that each group compiles with the features it is
  # published with; together they test every member of the workspace once.
  # --no-fail-fast: one failing test binary must not hide the rest.
  check_run rust-test "rust · cargo test of the daemon and its crates" --needs "$DAEMON_STEP" --cwd "$ROOT" \
    --report cargo -- cargo test --workspace --locked --no-fail-fast "${without_app[@]}"
  published_daemon_again rust-daemon rust rust-test
  check_run rust-test-app "rust · cargo test of the app" --needs "$DAEMON_STEP" --cwd "$ROOT" \
    --report cargo -- cargo test --locked --no-fail-fast "${app[@]}"
  check_run rust-test-pruebas "rust · orbit-core with the Orbit Pruebas identity" --needs env-rust --cwd "$ROOT" \
    --report cargo -- cargo test -p orbit-core --features pruebas --locked --no-fail-fast
  # orbit-app-core's tests write the TypeScript of the bridge.
  check_run rust-generated "rust · generated TypeScript up to date" --after rust-test-app --cwd "$ROOT" -- \
    python3 -B "$CHECK_DIR/generated.py" compare "$GENERATED_TYPES" "$LOG_DIR/generated.json"
}

# The hub publish guards (scripts/tests/test_hub_publish.py) build a release
# with the real uv offline, so the wheels of server/uv.lock must already be in
# uv's cache. env-hub is what fills it (with --install) or proves a synced
# server/.venv is there: without it, a fresh machine failed 11 guards with
# "Network connectivity is disabled" (CI, 2026-10-21).
run_guards_zone() {
  check_run guards "guards" --needs env-rust --needs env-hub --cwd "$ROOT" --report "unittest:$LOG_DIR/guards.json" -- \
    python3 -B "$CHECK_DIR/unittest_runner.py" scripts/tests "$LOG_DIR/guards.json"
}

run_hub_zone() {
  check_run hub-ruff "hub · ruff" --needs env-hub --cwd "$ROOT/server" -- \
    .venv/bin/ruff check orbit_hub orbit_mcp tests ../scripts/e2e-mac "$HARNESS_TEST"
  check_run hub-format "hub · format" --needs env-hub --cwd "$ROOT/server" -- \
    .venv/bin/ruff format --check orbit_hub orbit_mcp tests ../scripts/e2e-mac "$HARNESS_TEST"
  hub_pytest hub-pytest "hub · pytest with the e2e"
  # With the hub's pytest configuration (asyncio mode), named explicitly: the
  # module lives outside server/tests, which the hub's testpaths cover.
  check_run hub-harness "hub · dry run of the Mac's real-test harness" --needs env-hub --cwd "$ROOT/server" \
    --report "junit:$LOG_DIR/hub-harness.xml" -- \
    .venv/bin/python -m pytest -q -c pyproject.toml --rootdir . "--junitxml=$LOG_DIR/hub-harness.xml" \
    "$HARNESS_TEST"
}

run_ui_zone() {
  check_run env-ui "environment · interface (node, pnpm)" --cwd "$ROOT" -- check_ui_environment
  local desktop="$ROOT/apps/desktop"
  check_run ui-lint "interface · lint" --needs env-ui --cwd "$desktop" -- pnpm run lint
  check_run ui-typecheck "interface · typecheck" --needs env-ui --cwd "$desktop" -- pnpm run typecheck
  check_run ui-test "interface · vitest" --needs env-ui --cwd "$desktop" \
    --report "junit:$LOG_DIR/ui-test.xml" -- \
    pnpm run test --reporter=default --reporter=junit "--outputFile.junit=$LOG_DIR/ui-test.xml"
  check_run ui-build "interface · build" --needs env-ui --cwd "$desktop" -- pnpm run build
}

# ── Run ─────────────────────────────────────────────────────────────────────

check_init "$LOG_DIR"
if [[ -z "$CHECK_PLAN" ]]; then
  trap 'check_interrupted INT; exit 130' INT
  trap 'check_interrupted TERM; exit 143' TERM
  printf "Orbit's local gate · zones: %s · output of each step in %s\n" "${requested[*]}" "$LOG_DIR"
fi

if selected rust guards hub; then
  [[ -n "$CHECK_PLAN" ]] || discover_rust_toolchain
  check_run env-rust "environment · Rust ${RUST_CHANNEL:-unpinned}" --cwd "$ROOT" -- report_rust_environment
fi
if selected rust hub; then
  check_run daemon "build the daemon (before any test)" --needs env-rust --cwd "$ROOT" -- \
    "${DAEMON_BUILD[@]}"
fi
if selected rust; then run_rust_zone; fi
if selected guards hub; then
  check_run env-hub "environment · hub (server/.venv)" --cwd "$ROOT" -- check_hub_environment
fi
if selected guards; then run_guards_zone; fi
if selected hub; then run_hub_zone; fi
if selected ui; then run_ui_zone; fi

if check_finish; then
  exit 0
fi
exit 1
