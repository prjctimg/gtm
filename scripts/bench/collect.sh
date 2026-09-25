#!/usr/bin/env bash

set -euo pipefail

TAG="${1:?usage: collect.sh <tag> [seconds]}"
SECONDS="${2:-30}"
case "${SECONDS}" in '' | *[!0-9]*) echo "seconds must be an integer" >&2; exit 2 ;; esac

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH_DIR="${REPO_DIR}/.bench"
FIXTURES_DIR="${REPO_DIR}/assets/fixtures"
RUN_BENCH="${REPO_DIR}/scripts/bench/run.sh"

mkdir -p "${BENCH_DIR}"

"${REPO_DIR}/scripts/bench/gen-fixtures.sh" -c >/dev/null || {
  echo "fixture validation failed — refusing to collect (see gen-fixtures.sh -c)" >&2
  exit 1
}

FILES=("${FIXTURES_DIR}/bench.flac" "${FIXTURES_DIR}/bench.mp3")
PLAYERS=(gtm)
if command -v cliamp >/dev/null 2>&1 || [ -n "${CLIAMP_BIN:-}" ]; then
  PLAYERS+=(cliamp)
else
  echo "warning: cliamp not found — running gtm only (set CLIAMP_BIN to include the reference player)" >&2
fi

RUNS="{}"
for player in "${PLAYERS[@]}"; do
  for file in "${FILES[@]}"; do
    name="$(basename "${file}")"
    key="${player}/${name}"
    line="$("${RUN_BENCH}" "${player}" "${file}" "${SECONDS}" || true)"
    if ! parsed="$(printf '%s' "${line}" | jq -c 'select(type == "object")' 2>/dev/null)" \
      || [ -z "${parsed}" ]; then
      echo "  ${key}: SKIPPED (no parseable result)" >&2
      continue
    fi
    err="$(printf '%s' "${parsed}" | jq -r '.error // empty')"
    if [ -n "${err}" ]; then
      echo "  ${key}: SKIPPED (error: ${err})" >&2
      continue
    fi
    if ! printf '%s' "${parsed}" | jq -e '.peak_rss_kb != null' >/dev/null 2>&1; then
      echo "  ${key}: SKIPPED (no parseable result)" >&2
      continue
    fi
    echo "  ${key}: $(printf '%s' "${parsed}" | jq -c '[.peak_rss_kb,.mean_rss_kb,.cpu_ms,.t_ready_ms]')" >&2
    RUNS="$(printf '%s' "${RUNS}" | jq --arg key "${key}" --argjson v "$(printf '%s' "${parsed}" | jq -c 'del(.player,.file,.error)')" '.[$key] = $v')"
  done
done

if [ "$(printf '%s' "${RUNS}" | jq 'length')" -eq 0 ]; then
  echo "error: no benchmark runs collected (look for SKIPPED lines above)" >&2
  exit 1
fi

TAG_CLEAN="${TAG#v}"
COMMIT="${GITHUB_SHA:-$(git -C "${REPO_DIR}" rev-parse --short HEAD 2>/dev/null || echo unknown)}"
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

RESULT_FILE="${BENCH_DIR}/${TAG_CLEAN}.json"
jq -n \
  --arg tag "${TAG_CLEAN}" \
  --arg date "${DATE}" \
  --arg commit "${COMMIT}" \
  --arg seconds "${SECONDS}" \
  --argjson runs "${RUNS}" \
  '{tag:$tag,date:$date,commit:$commit,seconds:$seconds,runs:$runs}' \
  > "${RESULT_FILE}"

echo "wrote ${RESULT_FILE} (ephemeral; render.sh embeds it in BENCHMARK.md)"
