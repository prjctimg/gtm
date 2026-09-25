#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed 's/^version = "\(.*\)"/\1/')"

DRY_RUN=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN="--dry-run" ;;
    *)
      echo "Error: unknown argument '$arg'. Usage: $0 [--dry-run]" >&2
      exit 1
      ;;
  esac
done

echo "Publishing gtm v${VERSION}..."

if [ -n "${DRY_RUN}" ]; then
  echo "Dry run — verifying packaging, no uploads."
  echo "Registry state: gtm=$(cargo search gtm --limit 1 2>/dev/null | grep -o '"'"'=[^#]*'"'"' | head -1)"
fi

for attempt in 1 2 3 4 5 6; do
  out="$(cargo publish --locked ${DRY_RUN} -p gtm 2>&1)"
  if [ "$?" -eq 0 ]; then
    echo "   ✓ gtm published (v${VERSION})"
    exit 0
  fi
  if grep -q "already exists" <<<"${out}"; then
    echo "   ✓ gtm v${VERSION} already published, skipping"
    exit 0
  fi
  if grep -q "failed to select a version for the requirement" <<<"${out}"; then
    echo "   · index propagation pending; retrying in 30 s (${attempt}/6)..."
    sleep 30
    continue
  fi
  echo "${out}" >&2
  exit 1
done
exit 1