#!/usr/bin/env bash
# Keep gtmd's versioned gtm dependency aligned with the workspace version.
#
# cargo refuses to publish a path-only dependency, so gtmd names gtm's version
# explicitly. A release bump that touches Cargo.toml but misses gtmd/Cargo.toml
# leaves the two out of step; cargo publish then rejects the daemon after every
# target has been built. Run this from CI and the release tag job so the
# mismatch stops a push instead of a release.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed 's/^version = "\(.*\)"/\1/')"
gtm_req="$(sed -n 's/^gtm = { version = "\([^"]*\)".*/\1/p' gtmd/Cargo.toml)"

if [ "${gtm_req}" != "${VERSION}" ]; then
  echo "Error: gtmd/Cargo.toml depends on gtm ${gtm_req:-<path-only>}, but the workspace is ${VERSION}." >&2
  echo "       Update the dependency before publishing, or crates.io gets a gtmd that will not build." >&2
  exit 1
fi

echo "crate versions aligned: gtm ${VERSION}"
