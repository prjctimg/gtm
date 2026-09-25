#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
from="${1:?from-ref required (e.g. v0.2.7)}"
to="${2:?to-ref required (e.g. v0.2.73)}"
version="${3:?version required (e.g. 0.2.73)}"
out="${4:-$repo_root/artifacts/release-notes.md}"

cd "$repo_root"

repo_url="$(git config --get remote.origin.url | sed -E 's#(\.git)?$##; s#^git@github.com:#https://github.com/#; s#^git://#https://github.com/#')"

commits="$(git log --format='%H|%aN|%aE|%s' "${from}..${to}" 2>/dev/null || true)"
if [[ -z "$commits" && "$from" == "$to" ]]; then
  commits="$(git log -1 --format='%H|%aN|%aE|%s' "${to}" 2>/dev/null || true)"
fi
if [[ -z "$commits" ]]; then
  echo "warning: no commits between ${from}..${to}, writing an empty changelog" >&2
fi

prior_authors="$(git log --format='%aE' "${from}" 2>/dev/null | sort -u || true)"

mkdir -p "$(dirname "$out")"
{
  echo "# gtm ${version}"
  echo
  echo "> Generated ${to} against ${from}. Review before publishing."
  echo
  echo "## What's Changed"
  echo
  while IFS='|' read -r sha name email subject; do
    short="${sha:0:7}"
    link="$repo_url/commit/$sha"
    echo "- [\`$short\`]($link) ${subject}"
  done <<<"$commits"
  echo

  this_emails="$(printf '%s\n' "$commits" | cut -d'|' -f3 | sort -u || true)"
  new_names=()
  while IFS='|' read -r sha name email subject; do
    if ! grep -qF -- "$email" <<<"$prior_authors"; then
      if ! printf '%s\n' "${new_names[@]:-}" | grep -qF -- "$name" 2>/dev/null; then
        new_names+=("$name")
      fi
    fi
  done <<<"$commits"

  if [[ ${#new_names[@]} -gt 0 ]]; then
    echo "## New Contributors"
    echo
    for n in "${new_names[@]}"; do
      echo "* $n made their first contribution in this release."
    done
    echo
  fi
  echo "**Full changelog**: ${from}...${to:-$(git rev-parse --short "$to")}"
} > "$out"

ncommits="$(printf '%s\n' "$commits" | wc -l)"
echo "wrote $out (${#new_names[@]} new contributor(s), ${ncommits} commits)"