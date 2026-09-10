#!/usr/bin/env bash
# Collect benchmark results for one release tag into bench-results/<tag>.json
# and refresh bench-results/latest.json.
#
#   scripts/bench/collect.sh <tag> [seconds]
#
# Runs the harness for every player x fixture combination, aggregates the
# per-run JSON lines into the release file, and validates the fixture hashes
# against the pinned commits in gen-fixtures.sh.
#
# Requires: jq, and release gtm/gtmd binaries (gtm, gtmd on PATH or the
# workspace target/release), plus cliamp for the reference player (optional;
# its runs are skipped if cliamp is not installed, with a warning).

set -euo pipefail

TAG="${1:?usage: collect.sh <tag> [seconds]}"
SECONDS="${2:-30}"
case "${SECONDS}" in '' | *[!0-9]*) echo "seconds must be an integer" >&2; exit 2 ;; esac

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH_DIR="${REPO_DIR}/bench-results"
FIXTURES_DIR="${BENCH_DIR}/fixtures"
RUN_BENCH="${REPO_DIR}/scripts/bench/run-bench.sh"

mkdir -p "${BENCH_DIR}"

# Validate fixtures against the pinned hashes first.
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
    if printf '%s' "${line}" | jq -e '.error?' >/dev/null 2>&1; then
      echo "  ${key}: SKIPPED (error: $(printf '%s' "${line}" | jq -r '.error'))" >&2
      continue
    fi
    if ! printf '%s' "${line}" | jq -e '.peak_rss_kb != null' >/dev/null 2>&1; then
      echo "  ${key}: SKIPPED (no parseable result)" >&2
      continue
    fi
    echo "  ${key}: $(printf '%s' "${line}" | jq -c '[.peak_rss_kb,.mean_rss_kb,.cpu_ms,.p50_latency_kb,.p95_latency_kb]')" >&2
    RUNS="$(printf '%s' "${RUNS}" | jq --arg key "${key}" --argjson v "$(printf '%s' "${line}" | jq 'del(.player,.file,.error)')" '.[$key] = $v')"
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

cp "${RESULT_FILE}" "${BENCH_DIR}/latest.json"
echo "wrote ${RESULT_FILE}"
