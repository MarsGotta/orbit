#!/usr/bin/env bash
# Show which hub release is active, which is the previous one, every published
# release, whether orbit-hub.service and the Deimos profile run from `current`,
# the state of the two units, what Hermes says of Deimos's gateway, and which
# release the gateway's orbit-mcp runs. Reads only; changes nothing. It reads
# no Hermes configuration: the gateway's translator is seen in the process
# table. Exits 1 when hermes is missing or its status fails, after showing the
# rest.
#
#   status.sh
set -euo pipefail
# shellcheck source=scripts/hub/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[[ $# -eq 0 ]] || {
	printf 'Uso: %s\n' "${0##*/}" >&2
	exit 2
}

describe() {
	local version
	version=$(hub_link_version "$1")
	if [[ -z "$version" ]]; then
		printf 'ninguna'
	elif [[ -f "$ORBIT_HUB_BASE/releases/$version/RELEASE" ]]; then
		printf '%s (commit %.12s, publicada %s)' "$version" \
			"$(hub_release_field "$version" commit)" "$(hub_release_field "$version" published_at)"
	else
		printf '%s (incompleta: sin RELEASE)' "$version"
	fi
}

hub_say "Base: $ORBIT_HUB_BASE"
hub_say "Activa:   $(describe current)"
hub_say "Anterior: $(describe previous)"
hub_print_releases

binary="$ORBIT_HUB_BASE/current/.venv/bin/orbit-hub"
exec_start=$(hub_unit_property "$HUB_UNIT" ExecStart)
working=$(hub_unit_property "$HUB_UNIT" WorkingDirectory)
if [[ "$exec_start" == *"$binary "* && "$working" == "$ORBIT_HUB_BASE/current" ]]; then
	hub_say "$HUB_UNIT arranca desde $ORBIT_HUB_BASE/current."
else
	hub_say "$HUB_UNIT NO arranca desde $ORBIT_HUB_BASE/current (ExecStart: ${exec_start:-nada}; WorkingDirectory: ${working:-nada})."
fi
for unit in "$HUB_UNIT" "$SERVE_UNIT"; do
	state=$(hub_unit_active "$unit")
	hub_say "$unit: ${state:-sin respuesta}"
done

active=$(hub_link_version current)
hub_say "Gateway de Deimos (no es una unidad de systemd: un gateway de Hermes por host sirve a todos los perfiles):"
gateway_status=0
hub_show_gateway_status || gateway_status=$?
hub_describe_gateway "${active:-ninguna}" activa
hub_check_deimos_profile
exit $((gateway_status == 0 ? 0 : 1))
