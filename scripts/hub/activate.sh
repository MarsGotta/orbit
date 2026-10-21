#!/usr/bin/env bash
# Activate a published release of the hub, or go back to the previous one.
#
#   activate.sh <version>        e.g. activate.sh 0.3.12
#   activate.sh anterior         the release that was active before this one
#
# `anterior` («previous») keeps its Spanish name: the operator's own release
# scripts call it by that name.
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
	printf 'Usage: %s <version>|anterior\n' "${0##*/}" >&2
	exit 2
}
[[ $# -eq 1 && -n "$1" ]] || usage
requested=$1

[[ -d "$ORBIT_HUB_BASE/releases" ]] ||
	hub_fail "there are no published versions in $ORBIT_HUB_BASE/releases; publish one with $HUB_SCRIPTS_DIR/publish.sh."
hub_take_lock

if [[ "$requested" == anterior ]]; then
	version=$(hub_link_version previous)
	[[ -n "$version" ]] || hub_fail "there is no previous version: $ORBIT_HUB_BASE/previous does not exist (the version has not changed yet)."
else
	version=$requested
fi
hub_is_version "$version" || hub_fail "'$requested' is neither an X.Y.Z version nor 'anterior'."

problem=$(hub_release_problem "$version")
if [[ -n "$problem" ]]; then
	printf 'Error: %s cannot be activated: %s.\n' "$version" "$problem" >&2
	hub_print_releases >&2
	exit 1
fi

for link in current previous; do
	if [[ -e "$ORBIT_HUB_BASE/$link" && ! -L "$ORBIT_HUB_BASE/$link" ]]; then
		hub_fail "$ORBIT_HUB_BASE/$link exists and is not a symbolic link; it is left alone."
	fi
done

hub_require_unit_runs_from_current
hub_require_hermes

old=$(hub_link_version current)
if [[ "$old" == "$version" ]]; then
	hub_say "$version was already the active one; the services are restarted anyway."
else
	# previous first: if this stops between the two, previous names the
	# release that is still current, which is harmless.
	[[ -z "$old" ]] || hub_point_link previous "releases/$old"
	hub_point_link current "releases/$version"
	printf '%s %s (before %s)\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$version" "${old:-none}" \
		>>"$ORBIT_HUB_BASE/activations.log"
	hub_say "current → releases/$version (before ${old:-none})."
fi

hub_restart_services "$version" "$(hub_way_back "$version")"
hub_check_deimos_profile

hub_say "Activated $version."
previous=$(hub_link_version previous)
if [[ -n "$previous" ]]; then
	hub_say "To go back to $previous: $HUB_SCRIPTS_DIR/activate.sh $previous"
else
	hub_say "There is no previous version: to go back to the hub of the development checkout, see \"Storage and upgrades\" in $HUB_DOC."
fi
