#!/usr/bin/env bash
# Bump the project version across every file that references it.
# Usage: scripts/version.sh <new-version>   (e.g. scripts/version.sh 0.2.6)
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <new-version>" >&2
  exit 1
fi

NEW="$1"
if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must be X.Y.Z (got '$NEW')" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Workspace Cargo.toml — first 'version = "..." line is the workspace version.
sed -i -E '0,/^version = "[0-9.]+"$/s//version = "'"$NEW"'"/' "$ROOT/Cargo.toml"

# flake.nix
sed -i -E 's/(version = ")[0-9.]+(";)/\1'"$NEW"'\2/' "$ROOT/flake.nix"

# Termux package manifests (release pipeline + local make deb-termux)
sed -i -E 's/(Version: )[0-9.]+/\1'"$NEW"'/' "$ROOT/termux/gtm.yml"
sed -i -E 's/(Version: )[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/termux/gtm.yml"

# Arch PKGBUILD
sed -i -E 's/(pkgver=)[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/arch/PKGBUILD"

# Arch (gtmd) RPM spec
sed -i -E 's/(Version: )[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/rpm/gtmd.spec"
sed -i -E 's/( - )[0-9]+\.[0-9]+\.[0-9]+-1/\1'"$NEW"'-1/' "$ROOT/dist/rpm/gtmd.spec"

echo "Bumped version to $NEW in:"
echo "  Cargo.toml (workspace)"
echo "  flake.nix"
echo "  termux/gtm.yml"
echo "  dist/termux/gtm.yml"
echo "  dist/arch/PKGBUILD"
echo "Review the diff, then commit."
