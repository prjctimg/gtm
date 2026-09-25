#!/usr/bin/env bash
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

sed -i -E '0,/^version = "[0-9.]+"$/s//version = "'"$NEW"'"/' "$ROOT/Cargo.toml"

sed -i -E 's/(version = ")[0-9.]+(";)/\1'"$NEW"'\2/' "$ROOT/flake.nix"

sed -i -E 's/(Version: )[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/termux/gtm.yml"

sed -i -E 's/(pkgver=)[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/arch/PKGBUILD"

sed -i -E 's/(Version: )[0-9.]+/\1'"$NEW"'/' "$ROOT/dist/rpm/gtmd.spec"
sed -i -E 's/( - )[0-9]+\.[0-9]+\.[0-9]+-1/\1'"$NEW"'-1/' "$ROOT/dist/rpm/gtmd.spec"

echo "Bumped version to $NEW in:"
echo "  Cargo.toml (workspace version)"
echo "  flake.nix"
echo "  dist/termux/gtm.yml"
echo "  dist/arch/PKGBUILD"
echo "Review the diff, then commit."
