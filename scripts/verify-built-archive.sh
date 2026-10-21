#!/bin/bash
# Verify the archive this checkout just built, without naming it by hand.
#
# Every caller that spelled out the file name became another copy of the version:
# the workflow, the host build script and this repository's own instructions all
# looked for a package that a version bump had already renamed. Callers pass the
# output directory and the commit; the name is derived here, once.
#
# Usage: scripts/verify-built-archive.sh <output-dir> <commit> [architecture]
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "uso: scripts/verify-built-archive.sh <directorio> <commit> [arquitectura]" >&2
  exit 2
fi

output_dir=$1
commit=$2
architecture=${3:-${ORBIT_ARCH:-$(uname -m)}}
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# The same manifest the packager and the verifier read.
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -1)
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Cargo.toml no declara una versión legible" >&2
  exit 1
fi

archive="$output_dir/Orbit-$version-macos-$architecture.zip"
python3 -B "$repo/scripts/verify-macos-archive.py" "$archive" \
  --architecture "$architecture" --commit "$commit" \
  --checksum-file "$archive.sha256" \
  --manifest "$output_dir/build-manifest.txt"
