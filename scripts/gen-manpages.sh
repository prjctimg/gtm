#!/usr/bin/env bash
set -euo pipefail

outdir="${1:-.}"
mkdir -p "$outdir/man"

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$script_dir/.."

# Prefer gtm.spec/man/ when cloned alongside (CI or local dev),
# fall back to local docs/man/ for offline builds.
if [[ -d "$repo_root/../gtm.spec/man" ]]; then
  docs_dir="$repo_root/../gtm.spec/man"
  echo "Using canonical manpage sources from gtm.spec"
elif [[ -d "$repo_root/gtm.spec/man" ]]; then
  docs_dir="$repo_root/gtm.spec/man"
  echo "Using canonical manpage sources from gtm.spec"
else
  docs_dir="$repo_root/docs/man"
  echo "Warning: gtm.spec not found, using local docs/man/"
fi

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
