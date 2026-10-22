# shellcheck shell=bash
# Shared by the production hub scripts next to this file; each sources it first.
#
# Sourcing it only defines settings and functions, and refuses an unredirected
# test run (below). It creates, reads or restarts nothing by itself: the
# isolation tests source it with nothing after it and rely on that.
#
# The hub and the Deimos translator run from a release: a read-only folder
# built once from a commit, under $ORBIT_HUB_BASE/releases/<version>, with its
# own Python environment. $ORBIT_HUB_BASE/current is a symlink to the active
# one; the systemd unit and the Deimos profile both go through it.
#
# Hermes runs one gateway per host for every profile; Hermes owns how it is
# supervised. Manage it through its own command line, not the retired
# hermes-gateway-deimos.service (`hermes -p default gateway restart`).
# docs/self-hosting.md is the operator's side of this.

set -euo pipefail

ORBIT_HUB_DEFAULT_URL=http://127.0.0.1:8700

# --- Tests never reach the machine --------------------------------------------
#
# scripts/tests/hub_harness.py marks every test process with
# ORBIT_HUB_UNDER_TEST. Under that mark each thing these scripts could touch on
# the real machine must be redirected — the base, systemctl, journalctl, ps,
# hermes, the hub's URL and the Deimos profile — or the script stops here,
# before doing anything, with status 3. Without the mark this is a no-op.
# ps only reads, but what it reads is this machine's processes: a test whose
# result depended on them would be deciding on the host, not on its fixture.
hub_refuse_unredirected_test_run() {
	[[ -n "${ORBIT_HUB_UNDER_TEST:-}" ]] || return 0
	local problems=() tool found
	if [[ -z "${ORBIT_HUB_BASE:-}" ]]; then
		problems+=("ORBIT_HUB_BASE unset")
	elif [[ "$ORBIT_HUB_BASE" == /srv || "$ORBIT_HUB_BASE" == /srv/* ]]; then
		problems+=("ORBIT_HUB_BASE=$ORBIT_HUB_BASE is under /srv")
	fi
	for tool in systemctl journalctl ps; do
		found=$(command -v "$tool" || true)
		if [[ -z "$found" ]] || ! grep -q "ORBIT_HUB_FAKE_${tool^^}" "$found" 2>/dev/null; then
			problems+=("$tool is not the tests' fake: ${found:-not on the PATH}")
		fi
	done
	# A path that does not exist cannot reach Hermes, and is how the tests
	# check the message for a missing hermes; anything that exists must be
	# the fake.
	if [[ -z "${ORBIT_HUB_HERMES:-}" ]]; then
		problems+=("ORBIT_HUB_HERMES unset: it would be the real hermes (${HOME:-~}/.local/bin/hermes)")
	elif [[ -e "$ORBIT_HUB_HERMES" ]] && ! grep -q ORBIT_HUB_FAKE_HERMES "$ORBIT_HUB_HERMES" 2>/dev/null; then
		problems+=("ORBIT_HUB_HERMES=$ORBIT_HUB_HERMES is not the tests' fake")
	fi
	[[ -n "${ORBIT_HUB_URL:-}" ]] || problems+=("ORBIT_HUB_URL unset: it would be $ORBIT_HUB_DEFAULT_URL")
	[[ -n "${ORBIT_DEIMOS_PROFILE:-}" ]] || problems+=("ORBIT_DEIMOS_PROFILE unset: it would be the real Deimos profile")
	if ((${#problems[@]})); then
		printf 'Test without redirection: %s touches nothing.\n' "${0##*/}" >&2
		printf '  - %s\n' "${problems[@]}" >&2
		exit 3
	fi
}
hub_refuse_unredirected_test_run

# --- Settings ------------------------------------------------------------------

# Deployment paths are explicit; a public checkout cannot select a host.
: "${ORBIT_HUB_BASE:?ORBIT_HUB_BASE is missing}"
ORBIT_HUB_URL=${ORBIT_HUB_URL:-$ORBIT_HUB_DEFAULT_URL}
: "${ORBIT_DEIMOS_PROFILE:?ORBIT_DEIMOS_PROFILE is missing}"
# Hermes's command line, through which the gateway is restarted and asked about.
ORBIT_HUB_HERMES=${ORBIT_HUB_HERMES:-${HOME:?}/.local/bin/hermes}

# How long activation waits for the hub to answer /health once systemd has
# started it. Measured on 2026-09-25: a hub started from a read-only
# release on this server, with its working directory inside it and an empty
# store, answered /health in 0.68–1.73 s over 30 cold starts (median 0.80 s).
# Production opens a larger store and may start with a cold page cache, which
# that sandbox could not reproduce, so the budget is twenty times the worst
# observed start: 35 s. It is not how a crash is noticed — a hub that dies is
# seen at once, because systemd reports it (failed, or auto-restart) and the
# wait stops there. It only bounds a hub that stays alive without answering.
# It protects nothing and is no security limit; re-measure against the
# production journal if a real start ever comes close.
ORBIT_HUB_DEFAULT_HEALTH_TIMEOUT=35
ORBIT_HUB_HEALTH_TIMEOUT=${ORBIT_HUB_HEALTH_TIMEOUT:-$ORBIT_HUB_DEFAULT_HEALTH_TIMEOUT}

# How long activation waits, once `hermes -p default gateway restart` has
# answered, to see the gateway run an orbit-mcp of the release just activated.
# Neither the restart's answer nor the gateway's PID says it: on 2026-10-15 the
# restart of 0.4.0 answered "not confirmed: host operation is still pending",
# the gateway kept its PID (it was running since 10-10), and within a minute a
# new orbit-mcp child of it ran with releases/0.4.0's interpreter. That is one
# observation, "under a minute", so there is no distribution to take a p99
# from: the budget is twice that bound, 120 s. It only bounds a gateway that
# never respawns its translator; a restart Hermes refuses outright is reported
# at once. It protects nothing and is no security limit; re-measure (the new
# orbit-mcp's start time against the restart's) if a real one comes close.
ORBIT_HUB_DEFAULT_GATEWAY_TIMEOUT=120
ORBIT_HUB_GATEWAY_TIMEOUT=${ORBIT_HUB_GATEWAY_TIMEOUT:-$ORBIT_HUB_DEFAULT_GATEWAY_TIMEOUT}
# Seconds between two looks at the process table during that wait. One `ps`
# per second costs nothing and a respawn takes seconds; the tests, whose table
# is a file, set it lower only to run faster.
ORBIT_HUB_GATEWAY_POLL=${ORBIT_HUB_GATEWAY_POLL:-1}

HUB_UNIT=orbit-hub.service
SERVE_UNIT=hermes-serve-orbit.service
# Hermes manages the single host gateway through default; it serves Deimos
# and every other profile. Restarting it requires its operator's explicit approval.
GATEWAY_PROFILE=default
GATEWAY_NAME="the host's gateway (shared with Deimos)"
HUB_DOC=docs/self-hosting.md
HUB_SCRIPTS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# The copy of the database taken before each activation, its key, and any
# database an activation set aside (server/orbit_hub/backup.py).
HUB_BACKUPS=$ORBIT_HUB_BASE/backups
# Standard library only, so the server's python3 (3.11 or newer, for tomllib)
# runs it whichever release is active, or none.
HUB_BACKUP_SCRIPT=$HUB_SCRIPTS_DIR/../../server/orbit_hub/backup.py
ORBIT_HUB_PYTHON=${ORBIT_HUB_PYTHON:-python3}

# systemctl --user needs the user manager's bus. A shell opened from cron or SSH
# may not have it, and one opened from Claude Code has it set to something that
# is not a bus at all (measured on 2026-09-25: XDG_RUNTIME_DIR=/run/orca,
# DBUS_SESSION_BUS_ADDRESS=disabled:), which made the first activation refuse
# with "Failed to connect to user scope bus". So an inherited value is kept only
# when its bus socket exists; otherwise this user's own is used.
if [[ ! -S "${XDG_RUNTIME_DIR:-/nonexistent}/bus" ]]; then
	XDG_RUNTIME_DIR=/run/user/$(id -u)
fi
if [[ "${DBUS_SESSION_BUS_ADDRESS:-}" != unix:path=* || ! -S "${DBUS_SESSION_BUS_ADDRESS#unix:path=}" ]]; then
	DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
fi
export XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS

if [[ "$ORBIT_HUB_BASE" != /* ]]; then
	printf "Error: ORBIT_HUB_BASE must be an absolute path (it is '%s').\n" "$ORBIT_HUB_BASE" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_HEALTH_TIMEOUT" =~ ^[1-9][0-9]*$ ]]; then
	printf "Error: ORBIT_HUB_HEALTH_TIMEOUT must be a number of seconds (it is '%s').\n" \
		"$ORBIT_HUB_HEALTH_TIMEOUT" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_GATEWAY_TIMEOUT" =~ ^[1-9][0-9]*$ ]]; then
	printf "Error: ORBIT_HUB_GATEWAY_TIMEOUT must be a number of seconds (it is '%s').\n" \
		"$ORBIT_HUB_GATEWAY_TIMEOUT" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_GATEWAY_POLL" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
	printf "Error: ORBIT_HUB_GATEWAY_POLL must be a number of seconds (it is '%s').\n" \
		"$ORBIT_HUB_GATEWAY_POLL" >&2
	exit 2
fi

# --- Messages ------------------------------------------------------------------

hub_say() { printf '%s\n' "$*"; }
hub_warn() { printf 'Warning: %s\n' "$*" >&2; }
hub_fail() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

# --- Releases and links --------------------------------------------------------

hub_is_version() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

# The release a link (current, previous) points to, or nothing.
hub_link_version() {
	local target
	target=$(readlink -- "$ORBIT_HUB_BASE/$1" 2>/dev/null) || return 0
	printf '%s' "${target##*/}"
}

# One field of a release's RELEASE file (the first, if repeated), or nothing.
hub_release_field() {
	sed -n "/^$2=/{s///p;q;}" "$ORBIT_HUB_BASE/releases/$1/RELEASE" 2>/dev/null || true
}

# Why a version cannot be activated, or nothing if it can.
hub_release_problem() {
	local dir="$ORBIT_HUB_BASE/releases/$1" recorded name
	if [[ ! -d "$dir" ]]; then
		printf 'is not published (%s does not exist)' "$dir"
		return
	fi
	if [[ ! -f "$dir/RELEASE" ]]; then
		printf '%s is incomplete: it has no RELEASE, which is what an interrupted publication leaves' "$dir"
		return
	fi
	recorded=$(hub_release_field "$1" version)
	if [[ "$recorded" != "$1" ]]; then
		printf "%s/RELEASE says version '%s'" "$dir" "$recorded"
		return
	fi
	for name in orbit-hub orbit-mcp; do
		if [[ ! -x "$dir/.venv/bin/$name" ]]; then
			printf '%s lacks .venv/bin/%s' "$dir" "$name"
			return
		fi
	done
}

# Every published release, oldest version first, one line each.
hub_print_releases() {
	local dir version current previous marks names=()
	current=$(hub_link_version current)
	previous=$(hub_link_version previous)
	for dir in "$ORBIT_HUB_BASE"/releases/*/; do
		if [[ -d "$dir" ]]; then
			dir=${dir%/}
			names+=("${dir##*/}")
		fi
	done
	if ((${#names[@]} == 0)); then
		printf 'There are no published versions in %s.\n' "$ORBIT_HUB_BASE/releases"
		return
	fi
	printf 'Published versions in %s:\n' "$ORBIT_HUB_BASE/releases"
	while IFS= read -r version; do
		dir="$ORBIT_HUB_BASE/releases/$version"
		marks=""
		[[ "$version" != "$current" ]] || marks+="  ← current"
		[[ "$version" != "$previous" ]] || marks+="  ← previous"
		if [[ -f "$dir/RELEASE" ]]; then
			printf '  %-10s commit %.12s  published %s%s\n' "$version" \
				"$(hub_release_field "$version" commit)" "$(hub_release_field "$version" published_at)" "$marks"
		else
			printf '  %-10s incomplete (no RELEASE): cannot be activated%s\n' "$version" "$marks"
		fi
	done < <(printf '%s\n' "${names[@]}" | sort -V)
}

# Point <base>/<name> at <target> in one rename(2), so there is never an
# instant without it: `ln -sfn` unlinks first and would leave one.
hub_point_link() {
	local link="$ORBIT_HUB_BASE/$1" staged="$ORBIT_HUB_BASE/.$1.$$"
	rm -f -- "$staged"
	ln -s -- "$2" "$staged"
	mv -T -- "$staged" "$link"
}

# One mutating command at a time over the same base.
hub_take_lock() {
	[[ -d "$ORBIT_HUB_BASE" ]] || hub_fail "$ORBIT_HUB_BASE does not exist: nothing is published yet ($HUB_SCRIPTS_DIR/publish.sh)."
	exec 9>>"$ORBIT_HUB_BASE/.lock" || hub_fail "cannot open $ORBIT_HUB_BASE/.lock."
	flock -n 9 || hub_fail "another hub command is running on $ORBIT_HUB_BASE (it holds .lock); wait for it to finish."
}

# --- systemd -------------------------------------------------------------------

hub_systemctl() { systemctl --user "$@"; }

# A property's value, from stdout only: a warning systemd prints on stderr must
# not become part of it. On failure, ask again for the error to show.
hub_unit_property() {
	local value
	if value=$(hub_systemctl show "$1" "--property=$2" --value 2>/dev/null); then
		printf '%s' "$value"
		return 0
	fi
	hub_fail "could not ask systemd for $2 of $1: $(hub_systemctl show "$1" "--property=$2" --value 2>&1 >/dev/null || true)"
}

# is-active's answer (active, failed, activating…), from stdout only.
hub_unit_active() {
	hub_systemctl is-active "$1" 2>/dev/null || true
}

# Status and journal of a unit, to stderr, after a failure.
hub_report_unit() {
	printf '\n--- systemctl --user status %s ---\n' "$1" >&2
	hub_systemctl status "$1" --no-pager --lines=0 2>&1 | tail -n 20 >&2 || true
	printf -- '--- journalctl --user -u %s (last 30 lines) ---\n' "$1" >&2
	journalctl --user -u "$1" -n 30 --no-pager 2>&1 | tail -n 30 >&2 || true
}

hub_service_failure() {
	printf 'Error: %s\n' "$2" >&2
	hub_report_unit "$1"
	[[ -z "$3" ]] || printf '\n%s\n' "$3" >&2
	exit 1
}

# Activating means nothing unless the unit starts what `current` points to. A
# unit still on the development checkout, or edited without daemon-reload,
# would restart the old hub and let this say "activated".
hub_require_unit_runs_from_current() {
	local exec_start working reload problems=()
	local binary="$ORBIT_HUB_BASE/current/.venv/bin/orbit-hub"
	exec_start=$(hub_unit_property "$HUB_UNIT" ExecStart)
	working=$(hub_unit_property "$HUB_UNIT" WorkingDirectory)
	reload=$(hub_unit_property "$HUB_UNIT" NeedDaemonReload)
	[[ "$exec_start" == *"$binary "* ]] ||
		problems+=("ExecStart does not start $binary (it says: ${exec_start:-nothing})")
	[[ "$working" == "$ORBIT_HUB_BASE/current" ]] ||
		problems+=("WorkingDirectory is not $ORBIT_HUB_BASE/current (it says: ${working:-nothing})")
	[[ "$reload" == no ]] ||
		problems+=("systemd has not reread the unit (NeedDaemonReload=${reload:-?}): systemctl --user daemon-reload is missing")
	if ((${#problems[@]})); then
		printf 'Error: %s does not start from %s/current, so restarting it would not run the active version. Nothing was touched.\n' \
			"$HUB_UNIT" "$ORBIT_HUB_BASE" >&2
		printf '  - %s\n' "${problems[@]}" >&2
		printf 'Change the unit as %s says ("Install and configure the hub").\n' "$HUB_DOC" >&2
		exit 1
	fi
}

# What to run to undo an activation, for failure messages.
hub_way_back() {
	local previous
	previous=$(hub_link_version previous)
	if [[ -n "$previous" ]]; then
		printf 'The active version is now %s. To go back to %s:\n  %s/activate.sh %s' \
			"$1" "$previous" "$HUB_SCRIPTS_DIR" "$previous"
	else
		printf 'The active version is now %s and there is no earlier one. To go back to the hub of the development checkout, see "Upgrades" in %s.' \
			"$1" "$HUB_DOC"
	fi
}

hub_restart_unit() {
	local output status
	if output=$(hub_systemctl restart "$1" 2>&1); then
		return 0
	else
		status=$?
	fi
	hub_service_failure "$1" \
		"could not restart $1: systemctl --user restart exited with status $status: ${output:-no output}" "$2"
}

# Wait for the hub to answer /health, stopping at once if systemd says it died,
# then check it is the version just activated. Each /health attempt is capped at
# 2 s so a hub that accepts connections without answering still meets the
# deadline (a healthy one answers in milliseconds); attempts are 0.5 s apart,
# which is also how often systemd is asked whether the hub is still alive.
# /openapi.json gets 10 s: it is generated on the first request (9 ms measured
# for the full API) and is only read once.
hub_wait_for_hub() {
	local expected=$1 way_back=$2 deadline response code body last="" state sub reported
	deadline=$((SECONDS + ORBIT_HUB_HEALTH_TIMEOUT))
	while :; do
		if response=$(curl -sS --noproxy '*' --max-time 2 -w '\n%{http_code}' "$ORBIT_HUB_URL/health" 2>&1); then
			code=${response##*$'\n'}
			body=${response%$'\n'*}
			[[ "$code" != 200 || "$body" != *'"ok":true'* ]] || break
			last="HTTP $code: $body"
		else
			last=${response%$'\n'000}
		fi
		state=$(hub_unit_property "$HUB_UNIT" ActiveState)
		sub=$(hub_unit_property "$HUB_UNIT" SubState)
		if [[ "$state" == failed || "$state" == inactive || "$sub" == auto-restart ]]; then
			hub_service_failure "$HUB_UNIT" \
				"the hub ($HUB_UNIT) went down while starting: systemd says ActiveState=$state SubState=$sub. Last attempt at $ORBIT_HUB_URL/health: ${last:-none}. Neither $SERVE_UNIT nor $GATEWAY_NAME was restarted." \
				"$way_back"
		fi
		if ((SECONDS >= deadline)); then
			hub_service_failure "$HUB_UNIT" \
				"the hub ($HUB_UNIT) is still running ($state/$sub) but did not answer at $ORBIT_HUB_URL/health within $ORBIT_HUB_HEALTH_TIMEOUT s. Last answer: ${last:-none}. Neither $SERVE_UNIT nor $GATEWAY_NAME was restarted." \
				"$way_back"
		fi
		sleep 0.5
	done
	# /health says nothing about the version; the API description does.
	if ! response=$(curl -fsS --noproxy '*' --max-time 10 "$ORBIT_HUB_URL/openapi.json" 2>&1); then
		hub_warn "could not read the hub's version at $ORBIT_HUB_URL/openapi.json: $response"
		hub_say "  the hub answers at $ORBIT_HUB_URL/health; its version could not be checked."
		return 0
	fi
	reported=$(printf '%s' "$response" | python3 -c 'import json, sys; print(json.load(sys.stdin)["info"]["version"])' 2>/dev/null) || reported=""
	if [[ -z "$reported" ]]; then
		hub_warn "$ORBIT_HUB_URL/openapi.json does not say the hub's version."
		hub_say "  the hub answers at $ORBIT_HUB_URL/health; its version could not be checked."
	elif [[ "$reported" != "$expected" ]]; then
		hub_service_failure "$HUB_UNIT" \
			"the hub that answers says it is $reported, not the $expected just activated. Neither $SERVE_UNIT nor $GATEWAY_NAME was restarted." \
			"$way_back"
	else
		hub_say "  the hub answers at $ORBIT_HUB_URL/health and says it is $expected."
	fi
}

# --- The gateway, through Hermes ------------------------------------------------

# The command line for one gateway verb, as the operator would type it.
hub_gateway_command() {
	printf '%s -p %s gateway %s' "$ORBIT_HUB_HERMES" "$GATEWAY_PROFILE" "$1"
}

# Why hermes cannot be run, or nothing if it can.
hub_hermes_problem() {
	if [[ ! -e "$ORBIT_HUB_HERMES" ]]; then
		printf 'hermes not found at %s: it does not exist (ORBIT_HUB_HERMES changes the path)' "$ORBIT_HUB_HERMES"
	elif [[ ! -x "$ORBIT_HUB_HERMES" ]]; then
		printf 'hermes is at %s but is not executable (ORBIT_HUB_HERMES changes the path)' "$ORBIT_HUB_HERMES"
	fi
}

# Before anything moves: without hermes the gateway cannot be restarted, and an
# activation that stopped there would leave the hub changed and Deimos on the
# old translator — what happened on 2026-10-15 with the retired unit.
hub_require_hermes() {
	local problem
	problem=$(hub_hermes_problem)
	[[ -z "$problem" ]] ||
		hub_fail "$problem. Without it $GATEWAY_NAME cannot be restarted, so nothing was touched."
}

# This user's processes, "<pid> <ppid> <args>" one per line. The gateway runs
# as this user (measured: the operator's uid, child of their systemd --user, outside any
# unit). On failure, says why on stderr and returns 1.
hub_processes() {
	local listing status
	if listing=$(ps -U "$(id -u)" -o pid=,ppid=,args= 2>&1); then
		printf '%s\n' "$listing"
		return 0
	else
		status=$?
	fi
	printf 'ps -U %s -o pid=,ppid=,args= exited with status %s: %s\n' "$(id -u)" "$status" "${listing:-no output}" >&2
	return 1
}

# From a listing on stdin, every orbit-mcp whose parent is a gateway (a process
# whose arguments include `gateway run`), as "<pid> <gateway pid> <where>":
#   release:<version>  its interpreter is that release's: publish.sh's uv wrote
#                      it into the script's shebang, so ps shows it first;
#   current:           its interpreter is current's, which names no version;
#   other:<path>       its interpreter is from outside the base.
# Fully resolving the interpreter would not help: a release's .venv/bin/python
# is a link to the Python uv manages, shared by every release.
# hermes-serve-orbit's orbit-mcp hangs from a `serve` process and is left out.
hub_gateway_translators_in() {
	awk -v base="$ORBIT_HUB_BASE" '
		{
			pid = $1; ppid = $2
			$1 = ""; $2 = ""
			sub(/^ +/, "")
			args[pid] = $0; parent[pid] = ppid; order[++count] = pid
		}
		END {
			for (i = 1; i <= count; i++) {
				pid = order[i]
				split(args[pid], word, " ")
				if (word[1] !~ /\/orbit-mcp$/ && word[2] !~ /\/orbit-mcp$/) continue
				gateway = parent[pid]
				if (!(gateway in args) || args[gateway] !~ /(^| )gateway run( |$)/) continue
				path = word[1]
				if (index(path, base "/releases/") == 1) {
					rest = substr(path, length(base "/releases/") + 1)
					where = "release:" substr(rest, 1, index(rest "/", "/") - 1)
				} else if (index(path, base "/current/") == 1) {
					where = "current:"
				} else {
					where = "other:" path
				}
				print pid, gateway, where
			}
		}'
}

# Every process of this user whose arguments include `gateway run`, as
# "PID <pid> (<program>)", comma-separated. Containers' gateways show up too
# when they run as this user; they are named, not guessed away.
hub_gateway_processes_in() {
	awk '/(^| )gateway run( |$)/ { list = list sep "PID " $1 " (" $3 ")"; sep = ", " } END { print list }'
}

# A translator's <where>, in words, against the version it should be. With
# `active` as third argument, that version is named as the active one.
hub_describe_where() {
	local where=$1 wanted=$2 role=${3:-} matches mismatch
	if [[ "$role" == active ]]; then
		matches=", the active one"
		mismatch="not the active one ($wanted)"
	else
		matches=""
		mismatch="not $wanted"
	fi
	case "$where" in
	release:*)
		if [[ "${where#release:}" == "$wanted" ]]; then
			printf '%s%s' "$wanted" "$matches"
		else
			printf '%s, %s' "${where#release:}" "$mismatch"
		fi
		;;
	current:) printf 'from %s/current, without being able to say which version' "$ORBIT_HUB_BASE" ;;
	*) printf 'from outside %s (%s), %s' "$ORBIT_HUB_BASE" "${where#other:}" "$mismatch" ;;
	esac
}

# What can be seen of the gateway: its orbit-mcp children and their versions,
# or the gateway processes when none of them has one. To stdout. A second
# argument is passed on to hub_describe_where.
hub_describe_gateway() {
	local wanted=$1 role=${2:-} listing translators pid gateway where gateways
	if ! listing=$(hub_processes 2>&1); then
		printf '  could not read the process list: %s\n' "$listing"
		return 0
	fi
	translators=$(hub_gateway_translators_in <<<"$listing")
	if [[ -z "$translators" ]]; then
		gateways=$(hub_gateway_processes_in <<<"$listing")
		if [[ -n "$gateways" ]]; then
			printf "  no orbit-mcp hangs from a 'gateway run' process; this user's are: %s.\n" "$gateways"
		else
			printf "  no orbit-mcp hangs from a gateway: this user has no 'gateway run' process.\n"
		fi
		return 0
	fi
	while read -r pid gateway where; do
		printf "  the gateway's orbit-mcp: PID %s, child of gateway PID %s: %s.\n" \
			"$pid" "$gateway" "$(hub_describe_where "$where" "$wanted" "$role")"
	done <<<"$translators"
}

# Hermes's own word on the gateway, with its command and exit code, to stdout.
# Returns that exit code. Its text is shown, never interpreted: on 2026-10-15
# it printed "✗ Gateway is not running", exit 0, for Deimos while the host
# gateway was serving it.
hub_show_gateway_status() {
	local output status asked problem
	asked=$(hub_gateway_command status)
	problem=$(hub_hermes_problem)
	if [[ -n "$problem" ]]; then
		printf '  %s: %s.\n' "$asked" "$problem"
		return 127
	fi
	if output=$("$ORBIT_HUB_HERMES" -p "$GATEWAY_PROFILE" gateway status 2>&1); then
		status=0
	else
		status=$?
	fi
	printf '  %s (status %s):\n' "$asked" "$status"
	printf '%s\n' "${output:-no output}" | sed 's/^./    &/'
	return "$status"
}

hub_gateway_failure() {
	local message=$1 wanted=$2 way_back=$3
	printf 'Error: %s\n' "$message" >&2
	{
		printf "\n--- what can be seen of Deimos's gateway ---\n"
		hub_describe_gateway "$wanted"
		printf -- '--- what Hermes says ---\n'
		hub_show_gateway_status || true
		printf '\n%s\n' "$way_back"
		printf 'To retry only the gateway: %s\n' "$(hub_gateway_command restart)"
	} >&2
	exit 1
}

# The only answer of a failed-looking restart known to have worked (measured on
# 2026-10-15): Hermes gave up waiting for the confirmation, not the restart.
hub_gateway_restart_pending() {
	[[ "$1" == *"not confirmed"* || "$1" == *"still pending"* ]]
}

# Restart the gateway and wait until it runs an orbit-mcp of `expected` that it
# did not run before, which is the only proof it took the release: the PID of
# the gateway does not change and Hermes may not confirm. Without the "did not
# run before", restarting the same release would be proved by the old process.
hub_restart_gateway() {
	local expected=$1 way_back=$2 restart listing before output status deadline joined
	local pid gateway where found="" after="" others=()
	restart=$(hub_gateway_command restart)
	listing=$(hub_processes) ||
		hub_fail "could not read the process list before restarting $GATEWAY_NAME, so whether it takes $expected could not be known. It was not restarted. $way_back"
	before=$(hub_gateway_translators_in <<<"$listing")
	hub_say "Restarting $GATEWAY_NAME ($restart)…"
	if output=$("$ORBIT_HUB_HERMES" -p "$GATEWAY_PROFILE" gateway restart 2>&1); then
		status=0
	else
		status=$?
	fi
	if ((status != 0)) && ! hub_gateway_restart_pending "$output"; then
		hub_gateway_failure \
			"could not restart $GATEWAY_NAME: $restart exited with status $status: ${output:-no output}" \
			"$expected" "$way_back"
	fi
	deadline=$((SECONDS + ORBIT_HUB_GATEWAY_TIMEOUT))
	while :; do
		if listing=$(hub_processes); then
			after=$(hub_gateway_translators_in <<<"$listing")
			while read -r pid gateway where; do
				[[ "$where" == "release:$expected" ]] || continue
				[[ $'\n'"$before"$'\n' != *$'\n'"$pid "* ]] || continue
				found="$pid $gateway"
				break
			done <<<"$after"
			[[ -z "$found" ]] || break
		fi
		if ((SECONDS >= deadline)); then
			hub_gateway_failure \
				"$GATEWAY_NAME has not started a new orbit-mcp of $expected within $ORBIT_HUB_GATEWAY_TIMEOUT s after $restart, which exited with status $status: ${output:-no output}" \
				"$expected" "$way_back"
		fi
		sleep "$ORBIT_HUB_GATEWAY_POLL"
	done
	hub_say "  $GATEWAY_NAME (PID ${found#* }): new orbit-mcp, PID ${found%% *}, with $expected."
	if ((status != 0)) || hub_gateway_restart_pending "$output"; then
		hub_warn "$restart exited with status $status ('${output:-no output}'), but $GATEWAY_NAME already runs the orbit-mcp of $expected."
	fi
	while read -r pid gateway where; do
		[[ -n "$pid" && "$where" != "release:$expected" ]] || continue
		others+=("PID $pid, $(hub_describe_where "$where" "$expected")")
	done <<<"$after"
	if ((${#others[@]})); then
		printf -v joined '%s; ' "${others[@]}"
		hub_warn "the gateway also has other orbit-mcp processes that are not of $expected: ${joined%; }."
	fi
}

# --- The database around an activation -----------------------------------------

# The hub's configuration file, as its unit starts it (`--config <path>`).
hub_config_path() {
	local exec_start
	exec_start=$(hub_unit_property "$HUB_UNIT" ExecStart)
	if [[ "$exec_start" =~ --config[[:space:]]+([^[:space:];]+) ]]; then
		printf '%s' "${BASH_REMATCH[1]}"
	fi
}

# backup.py with the hub's configuration; its output, and its errors, to stdout.
hub_backup() {
	local config=$1
	shift
	"$ORBIT_HUB_PYTHON" -I "$HUB_BACKUP_SCRIPT" --config "$config" "$@" 2>&1
}

# Before the links move, from <old> to <version>. The hub migrates its
# database when a new release starts, and an older release's code must not
# run on a database migrated past it. So:
# - going to the release the last copy was taken from, with a schema that has
#   changed since: stop the hub and put that copy back; the database it
#   replaces, with everything written since, is moved aside, never deleted;
# - the same schema: nothing to do, the database keeps everything;
# - any other activation: first a consistent copy of the database and its key,
#   as <old> leaves them, in $HUB_BACKUPS/activation.
# A failure stops the activation before anything moved.
hub_guard_database() {
	local old=$1 version=$2 config plan output
	[[ "$old" != "$version" ]] || return 0
	config=$(hub_config_path)
	[[ -n "$config" ]] ||
		hub_fail "the ExecStart of $HUB_UNIT names no --config, so the database could not be copied before activating $version. Nothing was touched."
	[[ -r "$config" ]] ||
		hub_fail "cannot read the hub's configuration $config, so the database could not be copied before activating $version. Nothing was touched."
	plan=$(hub_backup "$config" activation-plan "$HUB_BACKUPS" --version "$version") ||
		hub_fail "could not compare the database with the copy in $HUB_BACKUPS/activation: $plan. Nothing was touched."
	case $plan in
	restore)
		hub_say "The database has another schema than when $version ran: putting back the copy taken when $version was left…"
		hub_say "Stopping $HUB_UNIT…"
		output=$(hub_systemctl stop "$HUB_UNIT" 2>&1) ||
			hub_service_failure "$HUB_UNIT" "could not stop $HUB_UNIT to put the copy back: ${output:-no output}. Nothing was moved." ""
		if ! output=$(hub_backup "$config" activation-restore "$HUB_BACKUPS"); then
			printf 'Error: could not put back the copy in %s: %s
' "$HUB_BACKUPS/activation" "$output" >&2
			if hub_systemctl start "$HUB_UNIT" >/dev/null 2>&1; then
				printf 'The database was left as it was and %s started again on %s. Nothing else was touched.
' "$HUB_UNIT" "${old:-the current version}" >&2
			else
				hub_report_unit "$HUB_UNIT"
				printf 'The database was left as it was, but %s did not start again: start it with systemctl --user start %s.
' "$HUB_UNIT" "$HUB_UNIT" >&2
			fi
			exit 1
		fi
		hub_say "$output"
		;;
	keep)
		hub_say "The database has the same schema as when $version ran: it keeps everything written since."
		;;
	copy)
		output=$(hub_backup "$config" activation-copy "$HUB_BACKUPS" --version "${old:-none}") ||
			hub_fail "could not copy the database before activating $version: $output. Nothing was touched."
		hub_say "$output"
		;;
	*)
		hub_fail "backup.py answered '$plan' about the database; nothing was touched."
		;;
	esac
}

# Hub first, then the two Hermes that each start their own orbit-mcp, which
# talks to the hub (docs/self-hosting.md, "Hermes and orbit-mcp").
hub_restart_services() {
	local expected=$1 way_back=$2 state
	hub_say "Restarting $HUB_UNIT…"
	hub_restart_unit "$HUB_UNIT" "$way_back"
	hub_wait_for_hub "$expected" "$way_back"
	hub_say "Restarting $SERVE_UNIT…"
	hub_restart_unit "$SERVE_UNIT" "$way_back"
	state=$(hub_unit_active "$SERVE_UNIT")
	[[ "$state" == active ]] ||
		hub_service_failure "$SERVE_UNIT" "$SERVE_UNIT is not active after restarting (is-active: ${state:-no answer}). $GATEWAY_NAME was not restarted." "$way_back"
	hub_say "  $SERVE_UNIT: active."
	hub_restart_gateway "$expected" "$way_back"
}

# The translator Deimos runs is named in its profile; activation cannot change
# that line (it is the profile owner's), but it can say when it points elsewhere.
hub_check_deimos_profile() {
	local line="command: $ORBIT_HUB_BASE/current/.venv/bin/orbit-mcp"
	if [[ ! -r "$ORBIT_DEIMOS_PROFILE" ]]; then
		hub_warn "could not read the Deimos profile ($ORBIT_DEIMOS_PROFILE) to see where orbit-mcp starts from."
	elif ! grep -Fq -- "$line" "$ORBIT_DEIMOS_PROFILE"; then
		hub_warn "the Deimos profile ($ORBIT_DEIMOS_PROFILE) does not start the translator from the active version, so Deimos keeps another orbit-mcp. The line must be '$line' ($HUB_DOC, \"Hermes and orbit-mcp\")."
	fi
}
