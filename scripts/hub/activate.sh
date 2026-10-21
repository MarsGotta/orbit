#!/usr/bin/env bash
# Activate a published release of the hub, or go back to the previous one.
#
#   activate.sh <versión>        e.g. activate.sh 0.3.12
#   activate.sh anterior         the release that was active before this one
#
# Points $ORBIT_HUB_BASE/current at the release in one rename, keeps the one it
# replaces as $ORBIT_HUB_BASE/previous, then restarts orbit-hub,
# hermes-serve-orbit and the shared host gateway (`hermes -p default gateway restart`)
# in that order. Nothing after the hub is touched until the hub answers /health
# as the version just activated, and the gateway counts as restarted only when
# it runs a new orbit-mcp of that version. On any failure it names the service,
# shows its output, status and journal (for the gateway: Hermes's command, its
# output and code, and the orbit-mcp it runs), and prints the command that goes
# back. It never goes back by itself.
#
# Before moving anything it checks that orbit-hub.service really starts from
# $ORBIT_HUB_BASE/current, otherwise "activated" would be untrue, and that
# hermes is there to restart the gateway.
set -euo pipefail
# shellcheck source=scripts/hub/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() {
	printf 'Uso: %s <versión>|anterior\n' "${0##*/}" >&2
	exit 2
}
[[ $# -eq 1 && -n "$1" ]] || usage
requested=$1

[[ -d "$ORBIT_HUB_BASE/releases" ]] ||
	hub_fail "no hay versiones publicadas en $ORBIT_HUB_BASE/releases; publica una con $HUB_SCRIPTS_DIR/publish.sh."
hub_take_lock

if [[ "$requested" == anterior ]]; then
	version=$(hub_link_version previous)
	[[ -n "$version" ]] || hub_fail "no hay versión anterior: $ORBIT_HUB_BASE/previous no existe (aún no se ha cambiado de versión)."
else
	version=$requested
fi
hub_is_version "$version" || hub_fail "«$requested» no es una versión X.Y.Z ni «anterior»."

problem=$(hub_release_problem "$version")
if [[ -n "$problem" ]]; then
	printf 'Error: no se puede activar la %s: %s.\n' "$version" "$problem" >&2
	hub_print_releases >&2
	exit 1
fi

for link in current previous; do
	if [[ -e "$ORBIT_HUB_BASE/$link" && ! -L "$ORBIT_HUB_BASE/$link" ]]; then
		hub_fail "$ORBIT_HUB_BASE/$link existe y no es un enlace simbólico; no lo toco."
	fi
done

hub_require_unit_runs_from_current
hub_require_hermes

old=$(hub_link_version current)
if [[ "$old" == "$version" ]]; then
	hub_say "La $version ya era la activa; se reinician los servicios igualmente."
else
	# previous first: if this stops between the two, previous names the
	# release that is still current, which is harmless.
	[[ -z "$old" ]] || hub_point_link previous "releases/$old"
	hub_point_link current "releases/$version"
	printf '%s %s (antes %s)\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$version" "${old:-ninguna}" \
		>>"$ORBIT_HUB_BASE/activations.log"
	hub_say "current → releases/$version (antes ${old:-ninguna})."
fi

hub_restart_services "$version" "$(hub_way_back "$version")"
hub_check_deimos_profile

hub_say "Activada la $version."
previous=$(hub_link_version previous)
if [[ -n "$previous" ]]; then
	hub_say "Para volver a la $previous: $HUB_SCRIPTS_DIR/activate.sh $previous"
else
	hub_say "No hay versión anterior: para volver al hub de la carpeta de desarrollo, «Volver al estado de hoy» en $HUB_DOC."
fi
