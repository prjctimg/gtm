#!/usr/bin/env bash
set -euo pipefail

# Regenerate CONTRIBUTORS.md from git commit authors.  Run manually from the
# repo root, or automatically by .github/workflows/contributors.yml on push.
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$repo_root/CONTRIBUTORS.md"

authors="$(git -C "$repo_root" log --format='%aN <%aE>' | sort -u | grep -v '^$')"
if [[ -z "$authors" ]]; then
  echo "Error: no commit authors found" >&2
  exit 1
fi

{
  cat <<'EOF'
# Contributors

People who have committed code to gtm.rs, automatically generated from git
history (see `scripts/build/contributors.sh`). New pull requests will be
listed here automatically.

EOF
  printf '%s\n' "$authors"
} > "$out"

echo "Wrote $out ($(printf '%s\n' "$authors" | wc -l | tr -d ' ') contributors)"
