#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
from="${1:?from-ref required}"
to="${2:?to-ref required}"
version="${3:?version required}"
out="${4:-$repo_root/artifacts/release-notes.md}"

cd "$repo_root"

repo_url="$(git config --get remote.origin.url | sed -E 's#(\.git)?$##; s#^git@github.com:#https://github.com/#; s#^git://#https://github.com/#')"

is_bot() {
  [[ "$2" =~ (\-bot|\[bot\])@ ]] && return 0
  [[ "$1" == *"[bot]"* ]] && return 0
  [[ "$3" == *"[skip ci]"* ]] && return 0
  [[ "$3" =~ ^(Merge|pull\ request\ from) ]] && return 0
  return 1
}

commit_type() {
  local re='^(feat|fix|perf|refactor|style|test|docs|build|ci|chore|revert)(\([^)]*\))?!?: '
  if [[ "$1" =~ $re ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  fi
}

type_heading() {
  case "$1" in
    feat) echo "Features" ;;
    fix) echo "Bug Fixes" ;;
    perf) echo "Performance" ;;
    refactor) echo "Refactoring" ;;
    revert) echo "Reverts" ;;
    test) echo "Tests" ;;
    docs) echo "Documentation" ;;
    build) echo "Build" ;;
    ci) echo "Continuous Integration" ;;
    style) echo "Style" ;;
    chore) echo "Chores" ;;
    *) echo "Other Changes" ;;
  esac
}

commits="$(git log --format='%H|%aN|%aE|%s' "${from}..${to}" 2>/dev/null || true)"
if [[ -z "$commits" && "$from" == "$to" ]]; then
  commits="$(git log -1 --format='%H|%aN|%aE|%s' "${to}" 2>/dev/null || true)"
fi

types=(feat fix perf refactor revert test docs build ci style chore other)
human=()
while IFS='|' read -r sha name email subject; do
  [[ -n "$sha" ]] || continue
  is_bot "$name" "$email" "$subject" && continue
  human+=("${sha}|${name}|${email}|${subject}")
done <<<"$commits"

if [[ ${#human[@]} -eq 0 ]]; then
  echo "warning: no human commits between ${from}..${to}" >&2
fi

prior_authors="$(git log --format='%aE' "$from" 2>/dev/null | sort -u || true)"

new_names=()
declare -A seen_new=()
for row in "${human[@]}"; do
  IFS='|' read -r _sha name email _subject <<<"$row"
  [[ -n "$email" ]] || continue
  grep -qxF -- "$email" <<<"$prior_authors" && continue
  [[ -n "${seen_new[$name]:-}" ]] && continue
  seen_new["$name"]=1
  new_names+=("$name")
done

mkdir -p "$(dirname "$out")"
{
  echo "# gtm ${version}"
  echo
  for t in "${types[@]}"; do
    section=()
    for row in "${human[@]}"; do
      IFS='|' read -r sha _name _email subject <<<"$row"
      kind="$(commit_type "$subject")"
      [[ -n "$kind" ]] || kind=other
      [[ "$kind" == "$t" ]] || continue
      section+=("- [\`${sha:0:7}\`](${repo_url}/commit/${sha}) ${subject#*: }")
    done
    [[ ${#section[@]} -gt 0 ]] || continue
    echo "## $(type_heading "$t")"
    echo
    printf '%s\n' "${section[@]}"
    echo
  done
  if [[ ${#new_names[@]} -gt 0 ]]; then
    echo "## New Contributors"
    echo
    for n in "${new_names[@]}"; do
      echo "* $n"
    done
    echo
  fi
  echo "**Full changelog**: ${repo_url}/compare/${from}...${to}"
} > "$out"

echo "wrote $out (${#new_names[@]} new contributor(s), ${#human[@]} commit(s))"