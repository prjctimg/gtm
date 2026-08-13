#!/usr/bin/env bash
set -euo pipefail

outdir="${1:-.}"
mkdir -p "$outdir/man"

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$script_dir/../.."

# Prefer gtm.spec/man/ when cloned alongside (CI or local dev),
# falling back to local docs/man/ for offline builds. Spec pages win;
# any manpage missing from the spec tree is filled in from docs/man/.
spec_dir=""
if [[ -d "$repo_root/../gtm.spec/man" ]]; then
  spec_dir="$repo_root/../gtm.spec/man"
  echo "Using canonical manpage sources from gtm.spec"
elif [[ -d "$repo_root/gtm.spec/man" ]]; then
  spec_dir="$repo_root/gtm.spec/man"
  echo "Using canonical manpage sources from gtm.spec"
else
  echo "No manpage sources in gtm.spec; falling back to docs/man/"
fi
docs_dir="$repo_root/docs/man"

if ! command -v pandoc &>/dev/null; then
  echo "Error: pandoc is not installed." >&2
  exit 1
fi

if [[ -n "$spec_dir" ]]; then
  for src in "$spec_dir"/*.1.md; do
    [[ -e "$src" ]] || continue
    name="$(basename "$src" .1.md)"
    echo "Generating manpage: $name.1"
    pandoc -s -t man "$src" -o "$outdir/man/$name.1"
  done
fi

# Fill any gaps (e.g. gtmd-ipc.1) from the local manpage sources.
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
  [[ -n "$spec_dir" ]] && echo "  - gtm.spec: $spec_dir" >&2 || echo "  - gtm.spec: (no man/ directory)" >&2
  echo "  - local:    $docs_dir" >&2
  echo "Place *.1.md files in one of these directories." >&2
  exit 1
fi
ls -1 "$outdir/man/"*.1
