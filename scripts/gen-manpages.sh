#!/usr/bin/env bash
set -euo pipefail

outdir="${1:-.}"
mkdir -p "$outdir/man"

script_dir="$(cd "$(dirname "$0")" && pwd)"
docs_dir="$script_dir/../docs/man"

if ! command -v pandoc &>/dev/null; then
  echo "Error: pandoc is not installed." >&2
  exit 1
fi

for src in "$docs_dir"/*.1.md; do
  name="$(basename "$src" .1.md)"
  echo "Generating manpage: $name.1"
  pandoc -s -t man "$src" -o "$outdir/man/$name.1"
done

echo "Manpages generated in $outdir/man/"
ls -1 "$outdir/man/"*.1
