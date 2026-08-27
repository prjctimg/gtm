#!/usr/bin/env bash
set -euo pipefail

outdir="${1:-.}"
mkdir -p "$outdir/man"

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$script_dir/../.."

docs_dir="$repo_root/docs/man"

if ! command -v pandoc &>/dev/null; then
  echo "Error: pandoc is not installed." >&2
  exit 1
fi

# Generate every manpage from the local docs/man sources, filling any gaps.
for src in "$docs_dir"/*.1.md; do
  [[ -e "$src" ]] || continue
  name="$(basename "$src" .1.md)"
  if [[ ! -f "$outdir/man/$name.1" ]]; then
    echo "Generating manpage (from local docs/man): $name.1"
    pandoc -s -t man "$src" -o "$outdir/man/$name.1"
  fi
done

echo "Manpages generated in $outdir/man/"
if [[ -z "$(ls -1 "$outdir/man/"*.1 2>/dev/null)" ]]; then
  echo "Error: no manpages were generated. Checked sources:" >&2
  echo "  - local:    $docs_dir" >&2
  echo "Place *.1.md files in this directory." >&2
  exit 1
fi
ls -1 "$outdir/man/"*.1
