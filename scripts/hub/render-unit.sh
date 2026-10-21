#!/usr/bin/env bash
# systemd cannot expand environment variables in executable or working-directory
# paths. Render the deployment's paths before installing the user unit.
set -euo pipefail
: "${ORBIT_HUB_BASE:?ORBIT_HUB_BASE is missing}"
: "${ORBIT_HUB_CONFIG:?ORBIT_HUB_CONFIG is missing}"
: "${ORBIT_HUB_ENV_FILE:?ORBIT_HUB_ENV_FILE is missing}"
for variable in ORBIT_HUB_BASE ORBIT_HUB_CONFIG ORBIT_HUB_ENV_FILE; do
  value=${!variable}
  if [[ ! "$value" =~ ^/[A-Za-z0-9_./-]+$ ]]; then
    printf '%s must be an absolute path without spaces or systemd characters: %s\n' "$variable" "$value" >&2
    exit 1
  fi
done
unit=$(cat "$(dirname "${BASH_SOURCE[0]}")/systemd/orbit-hub.service")
unit=${unit//@ORBIT_HUB_BASE@/$ORBIT_HUB_BASE}
unit=${unit//@ORBIT_HUB_CONFIG@/$ORBIT_HUB_CONFIG}
unit=${unit//@ORBIT_HUB_ENV_FILE@/$ORBIT_HUB_ENV_FILE}
printf '%s\n' "$unit"
