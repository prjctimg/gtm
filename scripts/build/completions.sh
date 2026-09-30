#!/usr/bin/env bash
# Generate shell completions for `gtm` and `gtmd` into <outdir>/completions.
#
# Each binary emits its own script from its own clap parser, so the scripts
# cannot drift from the flags the binaries accept. This used to be a
# hand-maintained copy of the argument structs in `gtm/build/completions.rs`,
# which is why the shipped scripts ended up missing flags the CLI had gained.
#
# The build is a prerequisite, not a side effect: packaging wants the scripts
# from the binaries it just built for that target, not from a second build.
set -euo pipefail

outdir="${1:?usage: completions.sh <outdir>}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"

# clap's zsh output is a completion function, so it is installed as `_gtm`,
# not under the shell's own name.
shells=(bash zsh fish elvish powershell)
names=(bash _ fish elv ps1)

for i in "${!shells[@]}"; do
  shell="${shells[$i]}"
  ext="${names[$i]}"
  for bin in gtm gtmd; do
    path="target/release/$bin"
    if [[ ! -x "$path" ]]; then
      echo "completions.sh: $path not built; run 'cargo build --release' first" >&2
      exit 1
    fi
    mkdir -p "$outdir/completions"
    "$path" --completions "$shell" >"$outdir/completions/$bin.$ext"
  done
done
