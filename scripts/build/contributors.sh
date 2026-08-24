#!/usr/bin/env bash
set -euo pipefail

# Regenerate contributors avatars in README.md from git commit authors.
# Run manually from repo root, or automatically by .github/workflows/contributors.yml on push.
#
# It collects unique authors (excluding bots), maps them to GitHub usernames,
# and injects a circle-avatar HTML block below the Acknowledgements section
# (between <!-- CONTRIBUTORS --> markers). Popular in many repos.
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
readme="$repo_root/README.md"

# ── Collect unique authors (name|email) ─────────────────────────────────
raw_authors="$(git -C "$repo_root" log --format='%aN|%aE' | sort -u | grep -v '^$' || true)"
if [[ -z "$raw_authors" ]]; then
  echo "Error: no commit authors found" >&2
  exit 1
fi

# Filter bots and deduplicate by username mapping
# Map email/name -> GitHub username
declare -A user_map
user_map["prjctimg@outlook.com"]="prjctimg"
user_map["prjctimg@yandex.com"]="skchr"
user_map["iseeheaven@outlook.com"]="iseeheaven"
# Also map by name fallback
user_map["prjctimg"]="prjctimg"
user_map["skchr"]="skchr"
user_map["iseeheaven"]="iseeheaven"

# Build list of github usernames
usernames=()
while IFS='|' read -r name email; do
  # skip bots
  if [[ "$name" == "github-actions[bot]" ]] || [[ "$email" == *"github-actions"* ]] || [[ "$email" == *"noreply.github.com"* ]]; then
    continue
  fi
  # resolve via email first, then name
  gh_user="${user_map[$email]:-}"
  if [[ -z "$gh_user" ]]; then
    gh_user="${user_map[$name]:-}"
  fi
  if [[ -z "$gh_user" ]]; then
    # fallback: sanitize name -> lowercase, strip spaces
    gh_user=$(echo "$name" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9-')
  fi
  # deduplicate
  found=0
  for u in "${usernames[@]:-}"; do
    if [[ "$u" == "$gh_user" ]]; then found=1; break; fi
  done
  if [[ $found -eq 0 && -n "$gh_user" ]]; then
    usernames+=("$gh_user")
  fi
done <<< "$raw_authors"

if [[ ${#usernames[@]} -eq 0 ]]; then
  echo "Error: no contributors after filtering" >&2
  exit 1
fi

# ── Generate avatar HTML (circle style) ─────────────────────────────────
avatars=""
for user in "${usernames[@]}"; do
  # Use GitHub avatar URL with size param, circle via border-radius
  avatars+="<a href=\"https://github.com/${user}\"><img src=\"https://github.com/${user}.png?size=80\" width=\"50\" height=\"50\" style=\"border-radius:50%;margin:4px;\" alt=\"${user}\"/></a> "
done
# Trim trailing space
avatars=$(echo "$avatars" | sed 's/ $//')

block="<!-- CONTRIBUTORS -->"
end="<!-- /CONTRIBUTORS -->"
section="

${block}
<p align=\"left\">
  ${avatars}
</p>
${end}
"

# ── Inject into README.md ───────────────────────────────────────────────
if grep -q "$block" "$readme"; then
  # Replace existing block
  # Use perl for multiline replace
  perl -0777 -i -pe "BEGIN{\$block=shift; \$end=shift; \$section=shift} s/\\Q\$block\\E.*?\\Q\$end\\E/\$section/s" "$block" "$end" "$section" "$readme" 2>/dev/null || {
    # fallback via ed-like
    # Recreate manually: remove old block lines, append new
    tmp=$(mktemp)
    awk -v blk="$block" -v end="$end" '
      $0==blk {p=1; print; next}
      $0==end {p=0; print; next}
      p==1 {next}
      {print}
    ' "$readme" > "$tmp" || true
    # If still contains block markers, replace content between them
    if grep -q "$block" "$tmp"; then
      # Use python for reliable multiline
      python3 - "$tmp" "$section" "$block" "$end" <<'PY'
import sys, pathlib
tmp, section, blk, end = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
text = pathlib.Path(tmp).read_text()
import re
pattern = re.compile(re.escape(blk) + ".*?" + re.escape(end), re.DOTALL)
new = re.sub(pattern, section.strip(), text)
pathlib.Path(tmp).write_text(new)
PY
    fi
    cat "$tmp" > "$readme"
    rm -f "$tmp"
  }
  # Ensure section is correctly placed if perl failed to fully replace
  if ! grep -q "github.com/${usernames[0]}.png" "$readme"; then
    # fallback append
    printf "%s\n" "$section" >> "$readme"
  fi
else
  # No existing block — append below Acknowledgements (or at end before copyright)
  # Find Acknowledgements section and append after it, else append at end
  if grep -q "## Acknowledgements" "$readme"; then
    # Insert after acknowledgements list, before the (c) line
    # Use awk to insert after last acknowledgements bullet
    tmp=$(mktemp)
    awk -v sec="$section" '
      BEGIN { inserted=0 }
      /^\(c\) 2026/ && !inserted { print sec; print ""; inserted=1 }
      { print }
      END { if (!inserted) print sec }
    ' "$readme" > "$tmp" && mv "$tmp" "$readme"
    # If (c) pattern not found, just append
    if ! grep -q "$block" "$readme"; then
      printf "%s\n" "$section" >> "$readme"
    fi
  else
    printf "%s\n" "$section" >> "$readme"
  fi
fi

# Also ensure old CONTRIBUTORS.md is removed or left deprecated note
if [[ -f "$repo_root/CONTRIBUTORS.md" ]]; then
  # Keep file but mark deprecated, or remove if workflow will stop using it
  # We leave removal to workflow; just note
  echo "Note: CONTRIBUTORS.md is deprecated — contributors now in README.md" >&2
fi

echo "Updated $readme with ${#usernames[@]} contributors: ${usernames[*]}"
