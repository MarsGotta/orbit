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
	printf 'Usage: %s\n' "${0##*/}" >&2
	exit 2
}

describe() {
	local version
	version=$(hub_link_version "$1")
	if [[ -z "$version" ]]; then
		printf 'none'
	elif [[ -f "$ORBIT_HUB_BASE/releases/$version/RELEASE" ]]; then
		printf '%s (commit %.12s, published %s)' "$version" \
			"$(hub_release_field "$version" commit)" "$(hub_release_field "$version" published_at)"
	else
		printf '%s (incomplete: no RELEASE)' "$version"
	fi
}

hub_say "Base: $ORBIT_HUB_BASE"
hub_say "Active:   $(describe current)"
hub_say "Previous: $(describe previous)"
hub_print_releases

binary="$ORBIT_HUB_BASE/current/.venv/bin/orbit-hub"
exec_start=$(hub_unit_property "$HUB_UNIT" ExecStart)
working=$(hub_unit_property "$HUB_UNIT" WorkingDirectory)
if [[ "$exec_start" == *"$binary "* && "$working" == "$ORBIT_HUB_BASE/current" ]]; then
	hub_say "$HUB_UNIT starts from $ORBIT_HUB_BASE/current."
else
	hub_say "$HUB_UNIT does NOT start from $ORBIT_HUB_BASE/current (ExecStart: ${exec_start:-nothing}; WorkingDirectory: ${working:-nothing})."
fi
for unit in "$HUB_UNIT" "$SERVE_UNIT"; do
	state=$(hub_unit_active "$unit")
	hub_say "$unit: ${state:-no answer}"
done

active=$(hub_link_version current)
hub_say "Deimos's gateway (not a systemd unit: one Hermes gateway per host serves every profile):"
gateway_status=0
hub_show_gateway_status || gateway_status=$?
hub_describe_gateway "${active:-none}" active
hub_check_deimos_profile
exit $((gateway_status == 0 ? 0 : 1))
