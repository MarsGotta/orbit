#!/usr/bin/env bash
# systemd cannot expand environment variables in executable or working-directory
# paths. Render the deployment's paths before installing a user unit.
#
#   render-unit.sh                              orbit-hub.service
#   render-unit.sh orbit-hub-backup.service     also needs ORBIT_HUB_BACKUP_DIR
#   render-unit.sh orbit-hub-backup.timer       has no paths
set -euo pipefail
name=${1:-orbit-hub.service}
template="$(dirname "${BASH_SOURCE[0]}")/systemd/$name"
if [[ "$name" == */* || ! -f "$template" ]]; then
  printf 'There is no unit %s in %s.\n' "$name" "$(dirname "$template")" >&2
  exit 2
fi
unit=$(cat "$template")
for variable in ORBIT_HUB_BASE ORBIT_HUB_CONFIG ORBIT_HUB_ENV_FILE ORBIT_HUB_BACKUP_DIR; do
  [[ "$unit" == *"@$variable@"* ]] || continue
  value=${!variable:-}
  if [[ -z "$value" ]]; then
    printf '%s is missing\n' "$variable" >&2
    exit 1
  fi
  if [[ ! "$value" =~ ^/[A-Za-z0-9_./-]+$ ]]; then
    printf '%s must be an absolute path without spaces or systemd characters: %s\n' "$variable" "$value" >&2
    exit 1
  fi
  unit=${unit//@$variable@/$value}
done
printf '%s\n' "$unit"
