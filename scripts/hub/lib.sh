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

# --- Tests never reach the machine (REF-05 A8) -------------------------------
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
		problems+=("ORBIT_HUB_BASE sin fijar")
	elif [[ "$ORBIT_HUB_BASE" == /srv || "$ORBIT_HUB_BASE" == /srv/* ]]; then
		problems+=("ORBIT_HUB_BASE=$ORBIT_HUB_BASE está bajo /srv")
	fi
	for tool in systemctl journalctl ps; do
		found=$(command -v "$tool" || true)
		if [[ -z "$found" ]] || ! grep -q "ORBIT_HUB_FAKE_${tool^^}" "$found" 2>/dev/null; then
			problems+=("$tool no es el falso de las pruebas: ${found:-no está en el PATH}")
		fi
	done
	# A path that does not exist cannot reach Hermes, and is how the tests
	# check the message for a missing hermes; anything that exists must be
	# the fake.
	if [[ -z "${ORBIT_HUB_HERMES:-}" ]]; then
		problems+=("ORBIT_HUB_HERMES sin fijar: sería el hermes real (${HOME:-~}/.local/bin/hermes)")
	elif [[ -e "$ORBIT_HUB_HERMES" ]] && ! grep -q ORBIT_HUB_FAKE_HERMES "$ORBIT_HUB_HERMES" 2>/dev/null; then
		problems+=("ORBIT_HUB_HERMES=$ORBIT_HUB_HERMES no es el falso de las pruebas")
	fi
	[[ -n "${ORBIT_HUB_URL:-}" ]] || problems+=("ORBIT_HUB_URL sin fijar: sería $ORBIT_HUB_DEFAULT_URL")
	[[ -n "${ORBIT_DEIMOS_PROFILE:-}" ]] || problems+=("ORBIT_DEIMOS_PROFILE sin fijar: sería el perfil real de Deimos")
	if ((${#problems[@]})); then
		printf 'Prueba sin redirección: %s no toca nada.\n' "${0##*/}" >&2
		printf '  - %s\n' "${problems[@]}" >&2
		exit 3
	fi
}
hub_refuse_unredirected_test_run

# --- Settings ------------------------------------------------------------------

# Deployment paths are explicit; a public checkout cannot select a host.
: "${ORBIT_HUB_BASE:?falta ORBIT_HUB_BASE}"
ORBIT_HUB_URL=${ORBIT_HUB_URL:-$ORBIT_HUB_DEFAULT_URL}
: "${ORBIT_DEIMOS_PROFILE:?falta ORBIT_DEIMOS_PROFILE}"
# Hermes's command line, through which the gateway is restarted and asked about.
ORBIT_HUB_HERMES=${ORBIT_HUB_HERMES:-${HOME:?}/.local/bin/hermes}

# How long activation waits for the hub to answer /health once systemd has
# started it. Measured for REF-05 (2026-09-25): a hub started from a read-only
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
# and every other profile. Restarting it requires Mars's explicit approval.
GATEWAY_PROFILE=default
GATEWAY_NAME="el gateway del host (compartido con Deimos)"
HUB_DOC=docs/self-hosting.md
HUB_SCRIPTS_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

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
	printf 'Error: ORBIT_HUB_BASE tiene que ser una ruta absoluta (es «%s»).\n' "$ORBIT_HUB_BASE" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_HEALTH_TIMEOUT" =~ ^[1-9][0-9]*$ ]]; then
	printf 'Error: ORBIT_HUB_HEALTH_TIMEOUT tiene que ser un número de segundos (es «%s»).\n' \
		"$ORBIT_HUB_HEALTH_TIMEOUT" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_GATEWAY_TIMEOUT" =~ ^[1-9][0-9]*$ ]]; then
	printf 'Error: ORBIT_HUB_GATEWAY_TIMEOUT tiene que ser un número de segundos (es «%s»).\n' \
		"$ORBIT_HUB_GATEWAY_TIMEOUT" >&2
	exit 2
fi
if [[ ! "$ORBIT_HUB_GATEWAY_POLL" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
	printf 'Error: ORBIT_HUB_GATEWAY_POLL tiene que ser un número de segundos (es «%s»).\n' \
		"$ORBIT_HUB_GATEWAY_POLL" >&2
	exit 2
fi

# --- Messages ------------------------------------------------------------------

hub_say() { printf '%s\n' "$*"; }
hub_warn() { printf 'Aviso: %s\n' "$*" >&2; }
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
		printf 'no está publicada (no existe %s)' "$dir"
		return
	fi
	if [[ ! -f "$dir/RELEASE" ]]; then
		printf '%s está incompleta: no tiene RELEASE, es lo que deja una publicación interrumpida' "$dir"
		return
	fi
	recorded=$(hub_release_field "$1" version)
	if [[ "$recorded" != "$1" ]]; then
		printf '%s/RELEASE dice versión «%s»' "$dir" "$recorded"
		return
	fi
	for name in orbit-hub orbit-mcp; do
		if [[ ! -x "$dir/.venv/bin/$name" ]]; then
			printf 'a %s le falta .venv/bin/%s' "$dir" "$name"
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
		printf 'No hay versiones publicadas en %s.\n' "$ORBIT_HUB_BASE/releases"
		return
	fi
	printf 'Versiones publicadas en %s:\n' "$ORBIT_HUB_BASE/releases"
	while IFS= read -r version; do
		dir="$ORBIT_HUB_BASE/releases/$version"
		marks=""
		[[ "$version" != "$current" ]] || marks+="  ← actual"
		[[ "$version" != "$previous" ]] || marks+="  ← anterior"
		if [[ -f "$dir/RELEASE" ]]; then
			printf '  %-10s commit %.12s  publicada %s%s\n' "$version" \
				"$(hub_release_field "$version" commit)" "$(hub_release_field "$version" published_at)" "$marks"
		else
			printf '  %-10s incompleta (sin RELEASE): no se puede activar%s\n' "$version" "$marks"
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
	[[ -d "$ORBIT_HUB_BASE" ]] || hub_fail "no existe $ORBIT_HUB_BASE: no hay nada publicado todavía ($HUB_SCRIPTS_DIR/publish.sh)."
	exec 9>>"$ORBIT_HUB_BASE/.lock" || hub_fail "no puedo abrir $ORBIT_HUB_BASE/.lock."
	flock -n 9 || hub_fail "otra orden del hub está en curso sobre $ORBIT_HUB_BASE (tiene .lock); espera a que termine."
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
	hub_fail "no pude preguntar a systemd por $2 de $1: $(hub_systemctl show "$1" "--property=$2" --value 2>&1 >/dev/null || true)"
}

# is-active's answer (active, failed, activating…), from stdout only.
hub_unit_active() {
	hub_systemctl is-active "$1" 2>/dev/null || true
}

# Status and journal of a unit, to stderr, after a failure.
hub_report_unit() {
	printf '\n--- systemctl --user status %s ---\n' "$1" >&2
	hub_systemctl status "$1" --no-pager --lines=0 2>&1 | tail -n 20 >&2 || true
	printf -- '--- journalctl --user -u %s (últimas 30 líneas) ---\n' "$1" >&2
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
		problems+=("ExecStart no arranca $binary (dice: ${exec_start:-nada})")
	[[ "$working" == "$ORBIT_HUB_BASE/current" ]] ||
		problems+=("WorkingDirectory no es $ORBIT_HUB_BASE/current (dice: ${working:-nada})")
	[[ "$reload" == no ]] ||
		problems+=("systemd no ha releído la unidad (NeedDaemonReload=${reload:-?}): falta systemctl --user daemon-reload")
	if ((${#problems[@]})); then
		printf 'Error: %s no arranca desde %s/current, así que reiniciarlo no pondría en marcha la versión activa. No se ha tocado nada.\n' \
			"$HUB_UNIT" "$ORBIT_HUB_BASE" >&2
		printf '  - %s\n' "${problems[@]}" >&2
		printf 'Cambia la unidad como dice %s («Primera vez»).\n' "$HUB_DOC" >&2
		exit 1
	fi
}

# What to run to undo an activation, for failure messages.
hub_way_back() {
	local previous
	previous=$(hub_link_version previous)
	if [[ -n "$previous" ]]; then
		printf 'La versión activa es ahora la %s. Para volver a la %s:\n  %s/activate.sh %s' \
			"$1" "$previous" "$HUB_SCRIPTS_DIR" "$previous"
	else
		printf 'La versión activa es ahora la %s y no hay otra anterior. Para volver al hub de la carpeta de desarrollo, sigue «Volver al estado de hoy» en %s.' \
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
		"no se pudo reiniciar $1: systemctl --user restart terminó con salida $status: ${output:-sin salida}" "$2"
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
				"el hub ($HUB_UNIT) se cayó al arrancar: systemd dice ActiveState=$state SubState=$sub. Último intento en $ORBIT_HUB_URL/health: ${last:-ninguno}. No se ha reiniciado $SERVE_UNIT ni $GATEWAY_NAME." \
				"$way_back"
		fi
		if ((SECONDS >= deadline)); then
			hub_service_failure "$HUB_UNIT" \
				"el hub ($HUB_UNIT) sigue en marcha ($state/$sub) pero no contestó en $ORBIT_HUB_URL/health en $ORBIT_HUB_HEALTH_TIMEOUT s. Última respuesta: ${last:-ninguna}. No se ha reiniciado $SERVE_UNIT ni $GATEWAY_NAME." \
				"$way_back"
		fi
		sleep 0.5
	done
	# /health says nothing about the version; the API description does.
	if ! response=$(curl -fsS --noproxy '*' --max-time 10 "$ORBIT_HUB_URL/openapi.json" 2>&1); then
		hub_warn "no pude leer la versión del hub en $ORBIT_HUB_URL/openapi.json: $response"
		hub_say "  el hub contesta en $ORBIT_HUB_URL/health; su versión no se ha podido comprobar."
		return 0
	fi
	reported=$(printf '%s' "$response" | python3 -c 'import json, sys; print(json.load(sys.stdin)["info"]["version"])' 2>/dev/null) || reported=""
	if [[ -z "$reported" ]]; then
		hub_warn "$ORBIT_HUB_URL/openapi.json no dice la versión del hub."
		hub_say "  el hub contesta en $ORBIT_HUB_URL/health; su versión no se ha podido comprobar."
	elif [[ "$reported" != "$expected" ]]; then
		hub_service_failure "$HUB_UNIT" \
			"el hub que contesta dice ser la $reported, no la $expected que se acaba de activar. No se ha reiniciado $SERVE_UNIT ni $GATEWAY_NAME." \
			"$way_back"
	else
		hub_say "  el hub contesta en $ORBIT_HUB_URL/health y dice ser la $expected."
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
		printf 'no encuentro hermes en %s: no existe (ORBIT_HUB_HERMES cambia la ruta)' "$ORBIT_HUB_HERMES"
	elif [[ ! -x "$ORBIT_HUB_HERMES" ]]; then
		printf 'hermes está en %s pero no es ejecutable (ORBIT_HUB_HERMES cambia la ruta)' "$ORBIT_HUB_HERMES"
	fi
}

# Before anything moves: without hermes the gateway cannot be restarted, and an
# activation that stopped there would leave the hub changed and Deimos on the
# old translator — what happened on 2026-10-15 with the retired unit.
hub_require_hermes() {
	local problem
	problem=$(hub_hermes_problem)
	[[ -z "$problem" ]] ||
		hub_fail "$problem. Sin él no se puede reiniciar $GATEWAY_NAME, así que no se ha tocado nada."
}

# This user's processes, "<pid> <ppid> <args>" one per line. The gateway runs
# as this user (measured: Mars's uid, child of her systemd --user, outside any
# unit). On failure, says why on stderr and returns 1.
hub_processes() {
	local listing status
	if listing=$(ps -U "$(id -u)" -o pid=,ppid=,args= 2>&1); then
		printf '%s\n' "$listing"
		return 0
	else
		status=$?
	fi
	printf 'ps -U %s -o pid=,ppid=,args= terminó con salida %s: %s\n' "$(id -u)" "$status" "${listing:-sin salida}" >&2
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
# `activa` as third argument, that version is named as the active one.
hub_describe_where() {
	local where=$1 wanted=$2 role=${3:-} matches mismatch
	if [[ "$role" == activa ]]; then
		matches=", la activa"
		mismatch="no la activa ($wanted)"
	else
		matches=""
		mismatch="no la $wanted"
	fi
	case "$where" in
	release:*)
		if [[ "${where#release:}" == "$wanted" ]]; then
			printf 'la %s%s' "$wanted" "$matches"
		else
			printf 'la %s, %s' "${where#release:}" "$mismatch"
		fi
		;;
	current:) printf 'desde %s/current, sin poder decir de qué versión' "$ORBIT_HUB_BASE" ;;
	*) printf 'de fuera de %s (%s), %s' "$ORBIT_HUB_BASE" "${where#other:}" "$mismatch" ;;
	esac
}

# What can be seen of the gateway: its orbit-mcp children and their versions,
# or the gateway processes when none of them has one. To stdout. A second
# argument is passed on to hub_describe_where.
hub_describe_gateway() {
	local wanted=$1 role=${2:-} listing translators pid gateway where gateways
	if ! listing=$(hub_processes 2>&1); then
		printf '  no pude leer la lista de procesos: %s\n' "$listing"
		return 0
	fi
	translators=$(hub_gateway_translators_in <<<"$listing")
	if [[ -z "$translators" ]]; then
		gateways=$(hub_gateway_processes_in <<<"$listing")
		if [[ -n "$gateways" ]]; then
			printf '  ningún orbit-mcp cuelga de un proceso «gateway run»; los de este usuario: %s.\n' "$gateways"
		else
			printf '  ningún orbit-mcp cuelga de un gateway: no hay ningún proceso «gateway run» de este usuario.\n'
		fi
		return 0
	fi
	while read -r pid gateway where; do
		printf '  orbit-mcp del gateway: PID %s, hijo del gateway PID %s: %s.\n' \
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
	printf '  %s (salida %s):\n' "$asked" "$status"
	printf '%s\n' "${output:-sin salida}" | sed 's/^./    &/'
	return "$status"
}

hub_gateway_failure() {
	local message=$1 wanted=$2 way_back=$3
	printf 'Error: %s\n' "$message" >&2
	{
		printf '\n--- lo que se ve del gateway de Deimos ---\n'
		hub_describe_gateway "$wanted"
		printf -- '--- lo que dice Hermes ---\n'
		hub_show_gateway_status || true
		printf '\n%s\n' "$way_back"
		printf 'Para reintentar sólo el gateway: %s\n' "$(hub_gateway_command restart)"
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
		hub_fail "no pude leer la lista de procesos antes de reiniciar $GATEWAY_NAME, así que no podría saber si coge la $expected. No se ha reiniciado. $way_back"
	before=$(hub_gateway_translators_in <<<"$listing")
	hub_say "Reiniciando $GATEWAY_NAME ($restart)…"
	if output=$("$ORBIT_HUB_HERMES" -p "$GATEWAY_PROFILE" gateway restart 2>&1); then
		status=0
	else
		status=$?
	fi
	if ((status != 0)) && ! hub_gateway_restart_pending "$output"; then
		hub_gateway_failure \
			"no se pudo reiniciar $GATEWAY_NAME: $restart terminó con salida $status: ${output:-sin salida}" \
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
				"$GATEWAY_NAME no ha arrancado un orbit-mcp nuevo de la $expected en $ORBIT_HUB_GATEWAY_TIMEOUT s tras $restart, que terminó con salida $status: ${output:-sin salida}" \
				"$expected" "$way_back"
		fi
		sleep "$ORBIT_HUB_GATEWAY_POLL"
	done
	hub_say "  $GATEWAY_NAME (PID ${found#* }): orbit-mcp nuevo, PID ${found%% *}, con la $expected."
	if ((status != 0)) || hub_gateway_restart_pending "$output"; then
		hub_warn "$restart terminó con salida $status («${output:-sin salida}»), pero $GATEWAY_NAME ya corre el orbit-mcp de la $expected."
	fi
	while read -r pid gateway where; do
		[[ -n "$pid" && "$where" != "release:$expected" ]] || continue
		others+=("PID $pid, $(hub_describe_where "$where" "$expected")")
	done <<<"$after"
	if ((${#others[@]})); then
		printf -v joined '%s; ' "${others[@]}"
		hub_warn "el gateway tiene además otros orbit-mcp que no son de la $expected: ${joined%; }."
	fi
}

# Hub first, then the two Hermes that each start their own orbit-mcp, which
# talks to the hub: see «Por qué este orden» in the operations document.
hub_restart_services() {
	local expected=$1 way_back=$2 state
	hub_say "Reiniciando $HUB_UNIT…"
	hub_restart_unit "$HUB_UNIT" "$way_back"
	hub_wait_for_hub "$expected" "$way_back"
	hub_say "Reiniciando $SERVE_UNIT…"
	hub_restart_unit "$SERVE_UNIT" "$way_back"
	state=$(hub_unit_active "$SERVE_UNIT")
	[[ "$state" == active ]] ||
		hub_service_failure "$SERVE_UNIT" "$SERVE_UNIT no quedó activo tras reiniciarse (is-active: ${state:-sin respuesta}). No se ha reiniciado $GATEWAY_NAME." "$way_back"
	hub_say "  $SERVE_UNIT: active."
	hub_restart_gateway "$expected" "$way_back"
}

# The translator Deimos runs is named in its profile; activation cannot change
# that line (it is Mars's), but it can say when it points elsewhere.
hub_check_deimos_profile() {
	local line="command: $ORBIT_HUB_BASE/current/.venv/bin/orbit-mcp"
	if [[ ! -r "$ORBIT_DEIMOS_PROFILE" ]]; then
		hub_warn "no pude leer el perfil de Deimos ($ORBIT_DEIMOS_PROFILE) para ver de dónde arranca orbit-mcp."
	elif ! grep -Fq -- "$line" "$ORBIT_DEIMOS_PROFILE"; then
		hub_warn "el perfil de Deimos ($ORBIT_DEIMOS_PROFILE) no arranca el traductor desde la versión activa, así que Deimos sigue con otro orbit-mcp. La línea tiene que ser «$line» ($HUB_DOC, «Primera vez»)."
	fi
}
