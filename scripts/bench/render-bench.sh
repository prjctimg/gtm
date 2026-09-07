#!/usr/bin/env bash
# Render BENCHMARK.md from committed bench-results/*.json.
#
#   scripts/bench/render-bench.sh [--tag <tag>] [-o BENCHMARK.md]
#
# Finds the newest *previously published* result as the comparison baseline and
# diffs the just-collected "this release" (the newest result, or <tag> when
# given) against it, producing:
#   - a per-metric delta table (vs last release)
#   - mermaid xychart: peak RSS bar (prev vs this) and mean-RSS trend line
#   - a results index table
#
# Requires: jq

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH_DIR="${REPO_DIR}/bench-results"
OUT="${REPO_DIR}/BENCHMARK.md"
THIS_TAG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) THIS_TAG="$2"; shift 2 ;;
    -o)    OUT="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

shopt -s nullglob
RESULTS=( "${BENCH_DIR}"/v[0-9]*.json "${BENCH_DIR}"/[0-9]*.json )
shopt -u nullglob
[ "${#RESULTS[@]}" -gt 0 ] || { echo "no bench-results/*.json found" >&2; exit 1; }

# Sort by mtime (approximates publication order); the newest file is "this".
IFS=$'\n' RESULTS_SORTED=( $(ls -t "${RESULTS[@]}") ); unset IFS
THIS="${RESULTS_SORTED[0]}"
if [ -n "${THIS_TAG}" ]; then
  cand="${BENCH_DIR}/${THIS_TAG#v}.json"
  [ -f "${cand}" ] && THIS="${cand}"
fi

PREV="${RESULTS_SORTED[1]:-}"

this_tag="$(jq -r '.tag' "${THIS}")"
this_date="$(jq -r '.date' "${THIS}")"
this_commit="$(jq -r '.commit' "${THIS}")"
prev_tag=""
[[ -n "${PREV}" && -f "${PREV}" ]] && prev_tag="$(jq -r '.tag' "${PREV}" 2>/dev/null || true)"

metric() { # file player/sample metric -> value
  jq -r --arg k "$2" --arg m "$3" '.runs[$k][$m] // 0' "$1" 2>/dev/null || echo 0
}

# Which samples exist in "this" (union of gtm + cliamp, flac + mp3).
SAMPLES="$(jq -r '.runs | keys[]' "${THIS}" 2>/dev/null || echo gtm/bench.flac)"

num() { # format an integer with thousands separators
  local n="${1:-0}"
  printf "%'d" "${n}" 2>/dev/null || echo "${n}"
}

# ── delta table ────────────────────────────────────────────────────────────────
TABLE=""
for sample in ${SAMPLES}; do
  for m in peak_rss_kb mean_rss_kb cpu_ms rss_5s_kb; do
    local_name="${sample%%/*}"
    local_file="${sample#*/}"
    this_v="$(metric "${THIS}" "${sample}" "${m}")"
    prev_v=0
    if [ -n "${prev_tag}" ]; then
      prev_v="$(metric "${PREV}" "${sample}" "${m}")"
    fi
    if [ "${prev_v}" -eq 0 ] 2>/dev/null && [ "${this_v}" -eq 0 ] 2>/dev/null; then
      continue
    fi
    label="${m}"
    case "${m}" in
      peak_rss_kb) label="peak RSS (kB)" ;;
      mean_rss_kb) label="mean RSS (kB)" ;;
      rss_5s_kb)   label="RSS @5s (kB)" ;;
      cpu_ms)      label="CPU (ms)" ;;
    esac
    d=$((this_v - prev_v))
    if [ "${d}" -gt 0 ]; then delta="**+$(num "${d}")**"
    elif [ "${d}" -lt 0 ]; then delta="**−$(num "$(( d * -1 ))")**"
    else delta="0"; fi
    TABLE+=$'\n'"| ${local_file} (${local_name}) | ${label} | $(num "${prev_v}") | $(num "${this_v}") | ${delta} |"
  done
done

# ── mermaid: peak RSS (prev vs this) per sample ────────────────────────────────
PEAK_BARS=""
for sample in ${SAMPLES}; do
  this_v="$(metric "${THIS}" "${sample}" peak_rss_kb)"
  prev_v=0; [ -n "${prev_tag}" ] && prev_v="$(metric "${PREV}" "${sample}" peak_rss_kb)"
  PEAK_BARS+=$'\n'"  bar [${prev_v}, ${this_v}]"
done

# ── mermaid: mean RSS trend for gtm (prev releases + this, oldest → newest) ────
TREND_PTS=""
TREND_LABELS=""
idx=0
for f in "${RESULTS_SORTED[@]}"; do
  [ "${f}" = "${THIS}" ] && continue
  [ "${idx}" -le 1 ] || break
  gtm_mean="$(jq -r '.runs["gtm/bench.flac"].mean_rss_kb // 0' "${f}" 2>/dev/null || echo 0)"
  TREND_PTS="${TREND_PTS} ${gtm_mean}"
  TREND_LABELS="${TREND_LABELS} \"$(jq -r '.tag' "${f}")\""
  idx=$((idx+1))
done
this_mean="$(jq -r '.runs["gtm/bench.flac"].mean_rss_kb // 0' "${THIS}" 2>/dev/null || echo 0)"
TREND_PTS="${TREND_PTS} ${this_mean}"
TREND_LABELS="${TREND_LABELS} \"${this_tag}\""
TREND_PTS="$(printf '%s' "${TREND_PTS}" | xargs | tr ' ' ',' )"
TREND_LABELS="$(printf '%s' "${TREND_LABELS}" | xargs | sed 's/ /, /g')"

# ── history index table ────────────────────────────────────────────────────────
IDX=""
for f in "${RESULTS_SORTED[@]}"; do
  t="$(jq -r '.tag' "${f}")"
  d="$(jq -r '.date' "${f}" | cut -dT -f1)"
  gpeak="$(jq -r '.runs["gtm/bench.flac"].peak_rss_kb // "-"' "${f}" 2>/dev/null || echo -)"
  IDX+=$'\n'"| ${t} | ${d} | ${gpeak} |"
done

this_tag_safe="${this_tag//|/}"
this_commit_safe="${this_commit//|/}"
this_date_safe="${this_date//|/}"
prev_tag_safe="${prev_tag//|/}"

if [ -z "${prev_tag}" ]; then
  baseline_note="No previous release results yet — the first benchmark establishes the baseline."
  table_prev_col="Prev"
  headline_suffix="(none yet)"
else
  baseline_note=""
  table_prev_col="Prev (\`${prev_tag_safe}\`)"
  headline_suffix="(\`${prev_tag_safe}\`)"
fi

doc="$(cat <<'EOF'
# gtm Benchmarks

Automated measurements of gtm's resource usage during playback, compared
release-over-release and against the reference CLI player **cliamp**
([bjarneo/cliamp](https://github.com/bjarneo/cliamp)).

> **This run**: release `__THIS_TAG__` (commit `__THIS_COMMIT__`, __THIS_DATE__).

## Headline — diff vs the last published release __HEADLINE_SUFFIX__

| Fixture (player) | Metric | __TABLE_PREV_COL__ | This | Δ |
|------------------|--------|------:|-----:|---:|__TABLE__

__BASELINE_NOTE__

## Peak RSS by release (gtm vs cliamp, kB)

```mermaid
xychart-beta
  title "Peak RSS by release (gtm vs cliamp, kB)"
  x-axis ["prev", "this"]
  y-axis "peak RSS (kB)" 0 --> 120000__PEAK_BARS__
```

## Mean RSS trend across releases (gtm, kB)

```mermaid
xychart-beta
  title "Mean RSS trend across releases (gtm, kB)"
  x-axis [__TREND_LABELS__]
  y-axis "mean RSS (kB)" 0 --> 60000
  line [__TREND_PTS__]
```

## History

| Release | Date | gtm peak RSS (kB) (FLAC) |
|---------|------|-----:|__IDX__

## Methodology

- A single representative FLAC and one MP3 (both committed under a sealed
  fixture hash — see `bench-results/fixtures/` and `gen-fixtures.sh -c`) are
  played for the same window for gtm and cliamp.
- gtm runs through its headless daemon in `--test-mode` (NullMixer, no audio
  device) so CPU/RSS are measured without an audio sink.
- Metrics: peak / mean / at-5s RSS (kB, from `/proc/<pid>/status` `VmRSS`),
  CPU (ms, from `/proc/<pid>/stat` utime+stime), and t_ready (ms, IPC round
  trip to first playing state — informational only).
- Harness: `scripts/bench/run-bench.sh <player> <file> <seconds>`; collection:
  `scripts/bench/collect.sh <tag>`; this renderer: `scripts/bench/render-bench.sh`.
EOF
)"

# Substitute placeholders; the doc is produced by us so these tokens are
# unique and never appear in the metric text.
doc="$(printf '%s' "${doc}" \
  | sed "s|__THIS_TAG__|${this_tag_safe}|g; s|__THIS_COMMIT__|${this_commit_safe}|g; s|__THIS_DATE__|${this_date_safe}|g; s|__HEADLINE_SUFFIX__|${headline_suffix}|g; s|__TABLE_PREV_COL__|${table_prev_col}|g; s|__BASELINE_NOTE__|${baseline_note}|g" \
  | python3 -c 'import sys; d=sys.stdin.read(); print(d.replace("__TABLE__", sys.argv[1]).replace("__PEAK_BARS__", sys.argv[2]).replace("__TREND_LABELS__", sys.argv[3]).replace("__TREND_PTS__", sys.argv[4]).replace("__IDX__", sys.argv[5]), end="")' "$(printf '%s' "${TABLE}" | sed -e 's|\\||g' -e 's/||/|/g')" "$(printf '%s' "${PEAK_BARS}" | sed -e 's|\\||g' -e 's/||/|/g')" "$(printf '%s' "${TREND_LABELS}")" "$(printf '%s' "${TREND_PTS}")" "$(printf '%s' "${IDX}" | sed -e 's|\\||g' -e 's/||/|/g')" )"

printf '%s\n' "${doc}" > "${OUT}"
echo "wrote ${OUT}"
