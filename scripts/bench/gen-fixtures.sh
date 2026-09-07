#!/usr/bin/env bash
# Generate the sealed benchmark fixtures (bench-results/fixtures/).
#
# A single representative FLAC and one MP3, committed only by hash: the files
# are deterministic outputs of ffmpeg (same seed → byte-identical), so the
# fixture hashes below pin them forever. A corrupt or regenerated fixture is
# caught because the harness echoes each file's sha256 into the results and
# the CI job compares against the committed expected hash.
#
# Usage:
#   scripts/bench/gen-fixtures.sh          # (re)generate into bench-results/fixtures
#   scripts/bench/gen-fixtures.sh -c       # verify existing fixtures match the pinned hashes
#
# Requires: ffmpeg

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURES_DIR="${REPO_DIR}/bench-results/fixtures"

# Pinned sha256 of the generated fixtures (see below). Change these together
# whenever the generator is intentionally changed.
FLAC_SHA="3e1f24d0c0ca4a7fa02de25bd240ed2ae409486f621b547c194a82612184209d"
MP3_SHA="d7a56cb8bbda7946fdd894ecd3165e039736473fef2817dba539188ff84edc55"

gen() {
  mkdir -p "${FIXTURES_DIR}"
  # Deterministic, non-trivial test tone: layered sine waves at integer
  # frequencies and a fixed 3-second length, so metadata + waveform are the
  # same on every run and across platforms.
  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:duration=3" \
    -metadata title="Bench Fixture" \
    -metadata artist="gtm benchmark" \
    -metadata album="bench" \
    -c:a flac "${FIXTURES_DIR}/bench.flac"

  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:duration=3" \
    -metadata title="Bench Fixture" \
    -metadata artist="gtm benchmark" \
    -metadata album="bench" \
    -codec:a libmp3lame -b:a 192k "${FIXTURES_DIR}/bench.mp3"
}

verify() {
  local rc=0
  for pair in "bench.flac:${FLAC_SHA}" "bench.mp3:${MP3_SHA}"; do
    local file="${pair%%:*}" want="${pair#*:}"
    local f="${FIXTURES_DIR}/${file}"
    if [ ! -f "${f}" ]; then
      echo "missing fixture: ${f}" >&2
      rc=1
      continue
    fi
    local got
    got="$(sha256sum "${f}" | awk '{print $1}')"
    if [ -n "${want}" ] && [ "${got}" != "${want}" ]; then
      echo "fixture hash mismatch: ${file}" >&2
      echo "  expected ${want}" >&2
      echo "  got      ${got}" >&2
      rc=1
    else
      echo "ok ${file} ${got}"
    fi
  done
  return "${rc}"
}

case "${1:-}" in
  -c | --check) verify ;;
  *) gen ; verify ;;
esac
