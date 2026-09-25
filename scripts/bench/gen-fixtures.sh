#!/usr/bin/env bash

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURES_DIR="${REPO_DIR}/assets/fixtures"

FLAC_SHA="c5e5c960cf59e5fdf3f3b68ea94a24d5f3d4b1122697e4e19d5ec5c1c3e175de"
MP3_SHA="6bd083a60ca94245ffd8d1be73e4b83eaab0fb6eeb6648e214ca528563a0bbfd"

gen() {
  mkdir -p "${FIXTURES_DIR}"
  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:duration=30" \
    -metadata title="Bench Fixture" \
    -metadata artist="gtm benchmark" \
    -metadata album="bench" \
    -c:a flac "${FIXTURES_DIR}/bench.flac"

  ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:duration=30" \
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
