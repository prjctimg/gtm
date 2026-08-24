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

# ── Inject into README.md ───────────────────────────────────────────────
# Single deterministic Python pass: replaces any existing block (and a
# previously generated heading) or inserts below Acknowledgements, keeping
# "## Contributors" above the avatars and clean spacing around it.
python3 - "$readme" "$avatars" <<'PY'
import pathlib, re, sys

readme, avatars = pathlib.Path(sys.argv[1]), sys.argv[2]
text = readme.read_text()
blk, end = "<!-- CONTRIBUTORS -->", "<!-- /CONTRIBUTORS -->"
section = (
    "## Contributors\n\n"
    + blk + '\n<p align="left">\n  ' + avatars + "\n</p>\n" + end
)
pat = re.compile(
    r"\n*(?:## Contributors\n+)?"
    + re.escape(blk) + r".*?" + re.escape(end) + r"\n*",
    re.DOTALL,
)
if pat.search(text):
    text = pat.sub(lambda _m: "\n\n" + section + "\n\n", text, count=1)
else:
    m = re.search(r"^(\(c\) .*)$", text, re.MULTILINE)
    if m:
        text = text[: m.start()] + section + "\n\n" + text[m.start():]
    else:
        text = text.rstrip("\n") + "\n\n" + section + "\n"
text = re.sub(r"\n{3,}", "\n\n", text)
if not text.endswith("\n"):
    text += "\n"
readme.write_text(text)
PY

# Also ensure old CONTRIBUTORS.md is removed or left deprecated note
if [[ -f "$repo_root/CONTRIBUTORS.md" ]]; then
  echo "Note: CONTRIBUTORS.md is deprecated — contributors now in README.md" >&2
fi

echo "Updated $readme with ${#usernames[@]} contributors: ${usernames[*]}"
