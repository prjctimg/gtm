#!/usr/bin/env bash
# Publish the gtm workspace crates to crates.io in dependency order.
#
# The crates share one workspace version and each internal dependency pins that
# exact version (`gtm-core = { path = ..., version = "X" }`). crates.io
# therefore requires every upstream crate to already be published at that
# version before the next one can go up. Publishing in any other order fails
# with:
#
#   failed to select a version for the requirement `gtm-core = "^X"`
#
# This script preflights each crate against the live crates.io index, waits for
# the upstream version to become visible (index propagation can lag the upload
# by ~30-60 s), then publishes. Already-published versions are skipped so a
# partial publish can be resumed.
#
# Usage:
#   scripts/build/publish.sh            # publish every crate (gtm-core first)
#   scripts/build/publish.sh --dry-run  # package + verify without uploading
#   scripts/build/publish.sh gtm-core   # publish a single crate (order enforced
#                                       # for its upstreams via preflight)
#
# Requires CARGO_REGISTRY_TOKEN (or a logged-in ~/.cargo/credentials).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed 's/^version = "\(.*\)"/\1/')"

DRY_RUN=""
CRATE_FILTER=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN="--dry-run" ;;
    *) CRATE_FILTER="$arg" ;;
  esac
done

# Crate -> whitespace-separated list of crates that must be live first.
prereqs() {
  case "$1" in
    gtm-core) echo "" ;;
    gtm-audio) echo "gtm-core" ;;
    gtm-mpris) echo "gtm-core" ;;
    gtmd) echo "gtm-core gtm-audio gtm-mpris" ;;
    gtm) echo "gtm-core gtm-audio" ;;
  esac
}

# Wait until `crate` is visible in the crates.io index at $VERSION.
# Index propagation can trail the upload by up to a few minutes.
wait_for_version() {
  local crate="$1"
  local attempts=0
  while true; do
    if cargo search "${crate}" --limit 1 2>/dev/null \
      | grep -q "(=\s*\"${VERSION}\")"; then
      echo "   ✓ ${crate} v${VERSION} visible on crates.io"
      return 0
    fi
    attempts=$((attempts + 1))
    if [ "${attempts}" -ge 20 ]; then
      echo "   ✗ ${crate} v${VERSION} still not visible after 20 polls" >&2
      return 1
    fi
    echo "   · waiting for ${crate} v${VERSION} index propagation (${attempts}/20)..."
    sleep 15
  done
}

publish_crate() {
  local crate="$1"
  echo "Publishing ${crate} v${VERSION}..."

  # Enforce dependency order against the live index (real mode only; a dry run
  # deliberately leaves the registry untouched).
  if [ -z "${DRY_RUN}" ]; then
    local need
    for need in $(prereqs "${crate}"); do
      wait_for_version "${need}"
      # Still a no-op if we already published `need` in this invocation, but a
      # re-run notices it straight away.
    done
  fi

  local attempt
  for attempt in 1 2 3 4 5 6; do
    local out
    if out="$(cargo publish --locked ${DRY_RUN} -p "${crate}" 2>&1)"; then
      echo "   ✓ ${crate} published (v${VERSION})"
      return 0
    fi
    if grep -q "already exists" <<<"${out}"; then
      echo "   ✓ ${crate} v${VERSION} already published, skipping"
      return 0
    fi
    if grep -q "failed to select a version for the requirement \`gtm-" <<<"${out}"; then
      echo "   · upstream just went live; retrying in 30 s (${attempt}/6)..."
      sleep 30
      continue
    fi
    echo "${out}" >&2
    return 1
  done
  return 1
}

CRATES="gtm-core gtm-audio gtm-mpris gtmd gtm"
if [ -n "${CRATE_FILTER}" ]; then
  case " ${CRATES} " in
    *" ${CRATE_FILTER} "*) CRATES="${CRATE_FILTER}" ;;
    *)
      echo "Error: unknown crate '${CRATE_FILTER}'. Valid: gtm-core gtm-audio gtm-mpris gtmd gtm" >&2
      exit 1
      ;;
  esac
fi

if [ -n "${DRY_RUN}" ]; then
  echo "Dry run — verifying packaging, no uploads."
  echo "Workspace version: ${VERSION}"
  echo "Registry state:    gtm-core=$(cargo search gtm-core --limit 1 2>/dev/null | grep -o '"'"'=[^#]*'"'"' | head -1)"
fi

for crate in ${CRATES}; do
  publish_crate "${crate}"
done
echo "Done."