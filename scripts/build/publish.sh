#!/usr/bin/env bash
# Publish the workspace to crates.io, in dependency order.
#
# Both crates ship: `gtm` (the library, TUI and CLI) and `gtmd` (the daemon).
# Publishing only `gtm` is what left crates.io serving a `gtmd 0.2.83` next to
# a `gtm 0.2.84` — a daemon a user resolved by version constraint could never
# build against the library that was actually out.
#
# Order matters: `gtmd` depends on `gtm`, so `gtm` goes first and the registry
# index has to catch up before `gtmd` is uploaded, which is what the
# "failed to select a version" retry below is for.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version = ' Cargo.toml | sed 's/^version = "\(.*\)"/\1/')"
CRATES=(gtm gtmd)

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

# `gtmd`'s dependency on `gtm` names a version of its own, because cargo
# refuses to publish a path-only dependency. Nothing keeps the two in step, so
# this is where it is checked: a mismatch publishes a `gtmd` that cannot
# resolve, and the failure only surfaces for whoever tries to depend on it.
gtm_req="$(sed -n 's/^gtm = { version = "\([^"]*\)".*/\1/p' gtmd/Cargo.toml)"
if [ "${gtm_req}" != "${VERSION}" ]; then
  echo "Error: gtmd/Cargo.toml depends on gtm ${gtm_req:-<path-only>}, but the workspace is ${VERSION}." >&2
  echo "       Update the dependency before publishing, or crates.io gets a gtmd that will not build." >&2
  exit 1
fi

echo "Publishing ${CRATES[*]} v${VERSION}..."

if [ -n "${DRY_RUN}" ]; then
  echo "Dry run — verifying packaging, no uploads."
  for c in "${CRATES[@]}"; do
    echo "  registry: $(cargo search "${c}" --limit 1 2>/dev/null | head -1)"
  done
fi

for crate in "${CRATES[@]}"; do
  published=0
  for attempt in 1 2 3 4 5 6; do
    set +e
    out="$(cargo publish --locked ${DRY_RUN} -p "${crate}" 2>&1)"
    status=$?
    set -e
    if [ "${status}" -eq 0 ]; then
      echo "   ✓ ${crate} published (v${VERSION})"
      published=1
      break
    fi
    if grep -q "already exists" <<<"${out}"; then
      if [ -n "${DRY_RUN}" ]; then
        # A dry run asks the registry whether the version is free and reports
        # "already exists" for one that is taken — which is the normal state
        # between a release and the next bump, and not a packaging failure.
        echo "   ✓ ${crate} v${VERSION} packages (version already on crates.io)"
      else
        echo "   ✓ ${crate} v${VERSION} already published, skipping"
      fi
      published=1
      break
    fi
    if grep -q "failed to select a version for the requirement" <<<"${out}"; then
      echo "   · ${crate}: index propagation pending; retrying in 30 s (${attempt}/6)..."
      sleep 30
      continue
    fi
    echo "${out}" >&2
    exit 1
  done
  if [ "${published}" -ne 1 ]; then
    echo "${crate} v${VERSION}: not published after 6 attempts" >&2
    exit 1
  fi
done