#!/usr/bin/env bash
# Publish the hub of one commit as a release: $ORBIT_HUB_BASE/releases/<version>.
#
#   publish.sh [<commit>]        (HEAD by default)
#
# A release is the server/ directory of that commit, without .git, with its own
# environment built by `uv sync --locked` from the commit's uv.lock, a RELEASE
# file saying version, commit and dates, and every file read-only. Publishing
# restarts nothing: activate.sh does that.
#
# It refuses, and says why, when server/ has uncommitted changes, when the
# commit's pyproject.toml and uv.lock disagree on the version, when the version
# is already published from another commit, and when the built environment
# cannot start orbit-hub, orbit-mcp or report its own version.
#
# Settings: ORBIT_HUB_BASE (required),
# ORBIT_REPO (default: the checkout this script is in), ORBIT_UV (default: uv on
# the PATH, then ~/.local/bin/uv).
set -euo pipefail
# shellcheck source=scripts/hub/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() {
	printf 'Uso: %s [<commit>]\n' "${0##*/}" >&2
	exit 2
}
[[ $# -le 1 ]] || usage
ref=${1:-HEAD}
[[ "$ref" != -* ]] || usage

repo=${ORBIT_REPO:-$(git -C "$HUB_SCRIPTS_DIR" rev-parse --show-toplevel 2>/dev/null || true)}
[[ -n "$repo" ]] && git -C "$repo" rev-parse --git-dir >/dev/null 2>&1 ||
	hub_fail "no encuentro el repositorio de Orbit (${repo:-sin ruta}); fija ORBIT_REPO."

uv=${ORBIT_UV:-$(command -v uv || printf '%s' "$HOME/.local/bin/uv")}
[[ -x "$uv" ]] || hub_fail "no encuentro uv ($uv); fija ORBIT_UV."

# --- What is being published ---------------------------------------------------

# A release comes from a commit, never from the working tree. A tree with
# changes in server/ is refused anyway: whoever publishes it most likely
# believes they are publishing what they see.
if ! dirty=$(GIT_OPTIONAL_LOCKS=0 git -C "$repo" status --porcelain --untracked-files=all -- server 2>&1); then
	hub_fail "git status falló en $repo: $dirty"
fi
if [[ -n "$dirty" ]]; then
	printf 'Error: hay cambios sin commitear en server/ de %s. Una versión sale de un commit, así que no publico:\n%s\n' \
		"$repo" "$dirty" >&2
	exit 1
fi

commit=$(git -C "$repo" rev-parse --verify --quiet --end-of-options "$ref^{commit}") ||
	hub_fail "no encuentro el commit «$ref» en $repo."
short=${commit:0:12}

pyproject=$(git -C "$repo" show "$commit:server/pyproject.toml" 2>/dev/null) ||
	hub_fail "el commit $short no tiene server/pyproject.toml."
lock=$(git -C "$repo" show "$commit:server/uv.lock" 2>/dev/null) ||
	hub_fail "el commit $short no tiene server/uv.lock."
pattern=$'(^|\n)version = "([0-9]+\\.[0-9]+\\.[0-9]+)"'
[[ "$pyproject" =~ $pattern ]] || hub_fail "server/pyproject.toml del commit $short no declara una versión X.Y.Z."
version=${BASH_REMATCH[2]}
pattern=$'(^|\n)name = "orbit-hub"\nversion = "([^"]*)"'
[[ "$lock" =~ $pattern ]] || hub_fail "server/uv.lock del commit $short no tiene la entrada de orbit-hub."
locked=${BASH_REMATCH[2]}
if [[ "$locked" != "$version" ]]; then
	hub_fail "en el commit $short, server/pyproject.toml dice $version y server/uv.lock dice $locked para orbit-hub: uv sync --locked no construiría esa versión. Cambia la versión con scripts/set-version.py, que escribe las dos, y commitea."
fi

releases="$ORBIT_HUB_BASE/releases"
target="$releases/$version"
mkdir -p -- "$releases" || hub_fail "no puedo crear $releases."
hub_take_lock

if [[ -e "$target" || -L "$target" ]]; then
	if [[ ! -f "$target/RELEASE" ]]; then
		hub_fail "$target existe pero está incompleta (no tiene RELEASE): es lo que deja una publicación interrumpida, y no la toco. Bórrala y vuelve a publicar: chmod -R u+w '$target' && rm -rf '$target'"
	fi
	published=$(hub_release_field "$version" commit)
	if [[ "$published" == "$commit" ]]; then
		hub_say "La $version ya está publicada desde este mismo commit ($short) en $target; no hay nada que hacer."
		exit 0
	fi
	hub_fail "la $version ya está publicada desde el commit ${published:0:12}, y este es $short. Una versión publicada no se rehace: cambia la versión con scripts/set-version.py, commitea y publica esa."
fi

# --- Build ---------------------------------------------------------------------

# Built in place, because a virtual environment records its own absolute path
# and cannot be moved afterwards. Until RELEASE is written the folder is not a
# release (activate.sh refuses it), and any failure removes it.
log=$(mktemp)
created=""
cleanup() {
	local status=$?
	if [[ -n "$created" ]]; then
		find "$created" ! -type l -exec chmod u+w {} + 2>/dev/null || true
		rm -rf -- "$created"
		printf 'Se ha borrado la publicación a medias de %s.\n' "$created" >&2
	fi
	rm -f -- "$log"
	exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# step <what> <command...>: run it with its output in $log; on failure, say
# which step and show the end of what it printed.
step() {
	local what=$1
	shift
	if ! "$@" >"$log" 2>&1; then
		printf 'Error: falló %s (en %s):\n' "$what" "$target" >&2
		tail -n 40 "$log" >&2
		exit 1
	fi
}

mkdir -- "$target"
created=$target
hub_say "Publicando la $version desde el commit $short en $target…"

extract_server() { git -C "$repo" archive --format=tar "$commit" server | tar -x -C "$target" --strip-components=1; }
step "copiar server/ del commit $short (git archive)" extract_server

# --link-mode copy: uv hardlinks from its cache by default, and a hardlinked
# file is the same file as the cache's and every other environment's (the
# development one included). Making the release read-only would change them,
# and changing them would change the release.
# Without --no-editable: the wheel hatchling builds carries only orbit_hub, not
# orbit_mcp; the editable install puts this release's own folder on the path.
build_environment() (
	cd "$target" && env -u VIRTUAL_ENV -u UV_PROJECT_ENVIRONMENT -u UV_FROZEN -u UV_NO_EDITABLE \
		"$uv" sync --locked --no-dev --link-mode copy --compile-bytecode
)
step "uv sync --locked (el entorno de la versión, desde el uv.lock del commit)" build_environment

python="$target/.venv/bin/python"
step "compilar orbit_hub y orbit_mcp" "$python" -I -m compileall -q "$target/orbit_hub" "$target/orbit_mcp"

# --- Check what was built ------------------------------------------------------

# Each check runs under `timeout 60`. Measured on this server (10 runs each):
# orbit-hub --help 0.17 s median, 0.62 s worst; orbit-mcp without configuration
# 0.60 s / 0.68 s; importing both and asking hub_version() 0.75 s / 0.78 s. The
# bound only stops a hung interpreter from hanging the publish; it limits
# nothing else. PYTHONPATH and PYTHONHOME are dropped so the operator's shell
# cannot lend the release a module it does not have.
clean_env=(env -u PYTHONPATH -u PYTHONHOME)
step "orbit-hub --help con el entorno de la versión" \
	"${clean_env[@]}" timeout 60 "$target/.venv/bin/orbit-hub" --help

# orbit-mcp has no --help and would serve stdio if it found a configuration.
# Pointed at one that does not exist it imports everything it serves with and
# stops with its safe startup message: that proves the entry point without
# starting anything or reading the real configuration.
mcp_status=0
mcp_output=$(ORBIT_MCP_CONFIG="$target/.sin-configuracion.toml" "${clean_env[@]}" \
	timeout 60 "$target/.venv/bin/orbit-mcp" </dev/null 2>&1) || mcp_status=$?
if [[ "$mcp_status" -ne 1 || "$mcp_output" != *"Orbit MCP no puede arrancar"* || "$mcp_output" == *Traceback* ]]; then
	printf 'Error: orbit-mcp no respondió como debe sin configuración (esperaba salida 1 y «Orbit MCP no puede arrancar»; salió %s):\n%s\n' \
		"$mcp_status" "$mcp_output" >&2
	exit 1
fi

reported=$("${clean_env[@]}" timeout 60 "$python" -I -c \
	'import orbit_mcp.server; from orbit_hub.app import hub_version; print(hub_version())' 2>&1) ||
	hub_fail "el entorno de la versión no importa el hub y el traductor:
$reported"
[[ "$reported" == "$version" ]] ||
	hub_fail "hub_version() del entorno construido dice «$reported» y la versión es $version."

python_version=$("$python" -I -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])')
commit_date=$(git -C "$repo" show -s --format=%cI "$commit")

# --- Seal ----------------------------------------------------------------------

{
	printf 'version=%s\n' "$version"
	printf 'commit=%s\n' "$commit"
	printf 'commit_date=%s\n' "$commit_date"
	printf 'published_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
	printf 'python=%s\n' "$python_version"
} >"$target/RELEASE.partial"
mv -- "$target/RELEASE.partial" "$target/RELEASE"
# Symlinks are left alone: .venv/bin/python points at the shared interpreter.
find "$target" ! -type l -exec chmod a-w {} +
created=""

hub_say "Publicada la $version (commit $short, Python $python_version) en $target."
hub_say "Para activarla: $HUB_SCRIPTS_DIR/activate.sh $version"
