#!/usr/bin/env bash
# Called before signing by every Orbit bundle build, including Orbit Pruebas.
set -euo pipefail
orbit_localization_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../native/macos/Resources" && pwd)"
orbit_resource_destination="${1:?Give the app bundle Resources directory}"
for orbit_language in en es; do
  mkdir -p -- "$orbit_resource_destination/$orbit_language.lproj"
  cp -- "$orbit_localization_root/$orbit_language.lproj/InfoPlist.strings" \
    "$orbit_localization_root/$orbit_language.lproj/Localizable.strings" \
    "$orbit_resource_destination/$orbit_language.lproj/"
done
