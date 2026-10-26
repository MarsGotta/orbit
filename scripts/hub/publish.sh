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
	printf 'Usage: %s [<commit>]\n' "${0##*/}" >&2
	exit 2
}
[[ $# -le 1 ]] || usage
ref=${1:-HEAD}
[[ "$ref" != -* ]] || usage

repo=${ORBIT_REPO:-$(git -C "$HUB_SCRIPTS_DIR" rev-parse --show-toplevel 2>/dev/null || true)}
[[ -n "$repo" ]] && git -C "$repo" rev-parse --git-dir >/dev/null 2>&1 ||
	hub_fail "cannot find the Orbit repository (${repo:-no path}); set ORBIT_REPO."

uv=${ORBIT_UV:-$(command -v uv || printf '%s' "$HOME/.local/bin/uv")}
[[ -x "$uv" ]] || hub_fail "cannot find uv ($uv); set ORBIT_UV."

# Probe the server's namespace policy before building any release. This does
# not contact a vendor, read a credential or change a shared service.
command -v bwrap >/dev/null || hub_fail "bubblewrap is required; install bubblewrap on the hub server."
if ! timeout 10 bwrap --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp \
    --unshare-pid --die-with-parent --new-session -- /bin/true; then
    hub_fail "bubblewrap cannot create a jail; allow user namespaces and review Ubuntu AppArmor's bwrap-userns-restrict profile."
fi

# --- What is being published ---------------------------------------------------

# A release comes from a commit, never from the working tree. A tree with
# changes in server/ is refused anyway: whoever publishes it most likely
# believes they are publishing what they see.
if ! dirty=$(GIT_OPTIONAL_LOCKS=0 git -C "$repo" status --porcelain --untracked-files=all -- server 2>&1); then
	hub_fail "git status failed in $repo: $dirty"
fi
if [[ -n "$dirty" ]]; then
	printf 'Error: there are uncommitted changes in server/ of %s. A version comes from a commit, so nothing is published:\n%s\n' \
		"$repo" "$dirty" >&2
	exit 1
fi

commit=$(git -C "$repo" rev-parse --verify --quiet --end-of-options "$ref^{commit}") ||
	hub_fail "cannot find the commit '$ref' in $repo."
short=${commit:0:12}

pyproject=$(git -C "$repo" show "$commit:server/pyproject.toml" 2>/dev/null) ||
	hub_fail "commit $short has no server/pyproject.toml."
lock=$(git -C "$repo" show "$commit:server/uv.lock" 2>/dev/null) ||
	hub_fail "commit $short has no server/uv.lock."
pattern=$'(^|\n)version = "([0-9]+\\.[0-9]+\\.[0-9]+)"'
[[ "$pyproject" =~ $pattern ]] || hub_fail "server/pyproject.toml of commit $short declares no X.Y.Z version."
version=${BASH_REMATCH[2]}
pattern=$'(^|\n)name = "orbit-hub"\nversion = "([^"]*)"'
[[ "$lock" =~ $pattern ]] || hub_fail "server/uv.lock of commit $short has no orbit-hub entry."
locked=${BASH_REMATCH[2]}
if [[ "$locked" != "$version" ]]; then
	hub_fail "in commit $short, server/pyproject.toml says $version and server/uv.lock says $locked for orbit-hub: uv sync --locked would not build that version. Change the version with scripts/release/set-version.py, which writes both, and commit."
fi

releases="$ORBIT_HUB_BASE/releases"
target="$releases/$version"
mkdir -p -- "$releases" || hub_fail "cannot create $releases."
hub_take_lock

if [[ -e "$target" || -L "$target" ]]; then
	if [[ ! -f "$target/RELEASE" ]]; then
		hub_fail "$target exists but is incomplete (it has no RELEASE): that is what an interrupted publication leaves, and it is left alone. Delete it and publish again: chmod -R u+w '$target' && rm -rf '$target'"
	fi
	published=$(hub_release_field "$version" commit)
	if [[ "$published" == "$commit" ]]; then
		hub_say "$version is already published from this same commit ($short) in $target; there is nothing to do."
		exit 0
	fi
	hub_fail "$version is already published from commit ${published:0:12}, and this one is $short. A published version is not rebuilt: change the version with scripts/release/set-version.py, commit and publish that."
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
		printf 'The half-done publication of %s was deleted.\n' "$created" >&2
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
		printf 'Error: %s failed (in %s):\n' "$what" "$target" >&2
		tail -n 40 "$log" >&2
		exit 1
	fi
}

mkdir -- "$target"
created=$target
hub_say "Publishing $version from commit $short in $target…"

extract_server() { git -C "$repo" archive --format=tar "$commit" server | tar -x -C "$target" --strip-components=1; }
step "copying server/ of commit $short (git archive)" extract_server

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
step "uv sync --locked (the version's environment, from the commit's uv.lock)" build_environment

python="$target/.venv/bin/python"
step "compiling orbit_hub and orbit_mcp" "$python" -I -m compileall -q "$target/orbit_hub" "$target/orbit_mcp"

# The pin travels with the committed server, so old releases never acquire a
# newer binary when this script changes. No activation or provider connection.
# Five minutes bounds the whole transfer, including a stalled release host.
step "downloading and verifying the pinned official Codex" \
	timeout 300 "$python" -I -m orbit_hub.assistant.codex_distribution "$target"
codex_version=$("$python" -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["version"])' "$target/codex-release.toml")
codex_sha256=$("$python" -I -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb"))["sha256"])' "$target/codex-release.toml")
# A private throwaway home prevents even a --version probe reading an account.
mkdir -m 700 "$target/.codex-check"
codex_reported=$(env -i PATH="$PATH" HOME="$target/.codex-check" CODEX_HOME="$target/.codex-check" \
	timeout 60 "$target/bin/codex" --version) || hub_fail "the shipped Codex cannot run on this platform: $target/bin/codex"
rm -rf -- "$target/.codex-check"
[[ "$codex_reported" == "codex-cli $codex_version" ]] || hub_fail "the shipped Codex reports '$codex_reported'; expected codex-cli $codex_version."

# --- Check what was built ------------------------------------------------------

# Each check runs under `timeout 60`. Measured on this server (10 runs each):
# orbit-hub --help 0.17 s median, 0.62 s worst; orbit-mcp without configuration
# 0.60 s / 0.68 s; importing both and asking hub_version() 0.75 s / 0.78 s. The
# bound only stops a hung interpreter from hanging the publish; it limits
# nothing else. PYTHONPATH and PYTHONHOME are dropped so the operator's shell
# cannot lend the release a module it does not have.
clean_env=(env -u PYTHONPATH -u PYTHONHOME)
step "orbit-hub --help with the version's environment" \
	"${clean_env[@]}" timeout 60 "$target/.venv/bin/orbit-hub" --help

# orbit-mcp has no --help and would serve stdio if it found a configuration.
# Pointed at one that does not exist it imports everything it serves with and
# stops with its safe startup message: that proves the entry point without
# starting anything or reading the real configuration. Releases up to 0.9.0
# word that message in Spanish, later ones in English.
mcp_status=0
mcp_output=$(ORBIT_MCP_CONFIG="$target/.no-configuration.toml" "${clean_env[@]}" \
	timeout 60 "$target/.venv/bin/orbit-mcp" </dev/null 2>&1) || mcp_status=$?
if [[ "$mcp_status" -ne 1 || ( "$mcp_output" != *"Orbit MCP cannot start"* && "$mcp_output" != *"Orbit MCP no puede arrancar"* ) || "$mcp_output" == *Traceback* ]]; then
	printf "Error: orbit-mcp did not answer as it must without configuration (expected status 1 and 'Orbit MCP cannot start'; it exited %s):\n%s\n" \
		"$mcp_status" "$mcp_output" >&2
	exit 1
fi

reported=$("${clean_env[@]}" timeout 60 "$python" -I -c \
	'import orbit_mcp.server; from orbit_hub.app import hub_version; print(hub_version())' 2>&1) ||
	hub_fail "the version's environment does not import the hub and the translator:
$reported"
[[ "$reported" == "$version" ]] ||
	hub_fail "hub_version() of the built environment says '$reported' and the version is $version."

python_version=$("$python" -I -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])')
commit_date=$(git -C "$repo" show -s --format=%cI "$commit")

# --- Seal ----------------------------------------------------------------------

{
	printf 'version=%s\n' "$version"
	printf 'commit=%s\n' "$commit"
	printf 'commit_date=%s\n' "$commit_date"
	printf 'published_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
	printf 'python=%s\n' "$python_version"
	printf 'codex=%s\n' "$codex_version"
	printf 'codex_asset_sha256=%s\n' "$codex_sha256"
} >"$target/RELEASE.partial"
mv -- "$target/RELEASE.partial" "$target/RELEASE"
# Symlinks are left alone: .venv/bin/python points at the shared interpreter.
find "$target" ! -type l -exec chmod a-w {} +
created=""

hub_say "Published $version (commit $short, Python $python_version) in $target."
hub_say "To activate it: $HUB_SCRIPTS_DIR/activate.sh $version"
