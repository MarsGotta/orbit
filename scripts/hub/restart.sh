#!/usr/bin/env bash
# Restart the hub and both Hermes in order, without changing the release.
#
#   restart.sh
#
# For after a change that is not a new release, such as editing orbit.toml or a
# `hermes update`: orbit-hub, then hermes-serve-orbit, then the shared host gateway
# (`hermes -p default gateway restart`), waiting for the hub's /health before
# the Hermes and for a new orbit-mcp of the active release under the gateway,
# with the same checks and failure report as activate.sh.
set -euo pipefail
# shellcheck source=scripts/hub/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[[ $# -eq 0 ]] || {
	printf 'Uso: %s\n' "${0##*/}" >&2
	exit 2
}

version=$(hub_link_version current)
[[ -n "$version" ]] ||
	hub_fail "no hay ninguna versión activa ($ORBIT_HUB_BASE/current no existe); activa una con $HUB_SCRIPTS_DIR/activate.sh <versión>."
hub_take_lock
problem=$(hub_release_problem "$version")
[[ -z "$problem" ]] || hub_fail "la versión activa, $version, no se puede arrancar: $problem."
hub_require_unit_runs_from_current
hub_require_hermes

hub_restart_services "$version" "$(hub_way_back "$version")"
hub_check_deimos_profile
hub_say "Reiniciados el hub, $SERVE_UNIT y $GATEWAY_NAME con la $version."
