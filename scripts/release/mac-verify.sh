#!/usr/bin/env bash
set -euo pipefail

release_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib.sh
source "$release_dir/lib.sh"
release_test_tools
archive=${1:-}
if [[ ! -f $archive ]]; then
  echo "The Mac's ZIP does not exist: '$archive'." >&2
  exit 1
fi
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT
ditto -x -k "$archive" "$scratch"
bundle="$scratch/Orbit.app"
echo 'codesign --verify --deep --strict'
codesign --verify --deep --strict "$bundle"
echo 'Signature accepted'
echo 'spctl --assess --type execute'
spctl --assess --type execute "$bundle"
echo 'Gatekeeper accepted'
echo 'xcrun stapler validate'
xcrun stapler validate "$bundle"
echo 'Stapling accepted'
