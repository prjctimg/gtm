#!/usr/bin/env bash
# Render BENCHMARK.md from benchmark results.
#
#   scripts/bench/render.sh [--tag <tag>] [-o BENCHMARK.md]
#
# Data flows:
#   - `scripts/bench/collect.sh` writes one ephemeral JSON per release under
#     the gitignored `.bench/` directory.
#   - render.sh merges those with history reconstructed from the
#     machine-readable `<!--bench:{json}-->` markers embedded at the bottom of
#     the committed BENCHMARK.md, producing:
#       - a per-metric delta table (vs the previous release)
#       - mermaid xychart: peak-RSS bars per release
#       - mermaid xychart: mean-RSS trend per release
#       - mermaid xychart: t_ready (ms) start-latency trend per release
#       - a results index table
#     and then writes the equivalent markers back into the doc, so the
#     committed BENCHMARK.md is the single permanent store (no JSON tracked).
#
# Requires: jq

set -euo pipefail

# Temp files from the doc render below are always removed, even on failure.
trap 'rm -f "${PYF:-}" "${DOC_TMP:-}"' EXIT

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH_DIR="${REPO_DIR}/.bench"
OUT="${REPO_DIR}/BENCHMARK.md"
THIS_TAG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) THIS_TAG="$2"; shift 2 ;;
    -o)    OUT="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "${BENCH_DIR}"
HIST_DIR="${BENCH_DIR}/history"
rm -rf "${HIST_DIR}"
mkdir -p "${HIST_DIR}"

# ── 1. Reconstruct history ────────────────────────────────────────────────────
# Markers in the committed BENCHMARK.md are the persistent store; any fresh
# `.bench/*.json` written by collect.sh overrides its own tag.
if [ -f "${OUT}" ]; then
  while IFS= read -r m; do
    [ -n "${m}" ] || continue
    json="${m#<!--bench:}"
    json="${json%-->}"
    tag="$(printf '%s' "${json}" | jq -r '.tag // empty' 2>/dev/null || true)"
    [ -n "${tag}" ] || continue
    printf '%s\n' "${json}" > "${HIST_DIR}/${tag}.json"
  done < <(grep -ho '<!--bench:{.*}-->' "${OUT}" 2>/dev/null || true)
fi

# Fresh .bench/*.json results override marker copies of the same tag.
for f in "${BENCH_DIR}"/*.json; do
  [ -f "${f}" ] || continue
  tag="$(jq -r '.tag // empty' "${f}" 2>/dev/null || true)"
  [ -n "${tag}" ] || continue
  cp "${f}" "${HIST_DIR}/${tag}.json"
done

shopt -s nullglob
HIST_FILES=( "${HIST_DIR}"/*.json )
shopt -u nullglob
[ "${#HIST_FILES[@]}" -gt 0 ] || {
  echo "no benchmark results found — run scripts/bench/collect.sh <tag> first" >&2
  exit 1
}

# Sort history by date (ISO 8601 strings compare lexically).
TAG_ORDER="$(jq -sr 'sort_by(.date) | .[].tag' "${HIST_FILES[@]}")"

file_for() { printf '%s/%s.json' "${HIST_DIR}" "$1"; }

# "this" = --tag if given, else the newest dated release.
THIS=""
if [ -n "${THIS_TAG}" ]; then
  for t in ${TAG_ORDER}; do
    if [ "${t}" = "${THIS_TAG}" ] && [ -f "$(file_for "${t}")" ]; then
      THIS="$(file_for "${t}")"
    fi
  done
fi
if [ -z "${THIS}" ]; then
  for t in ${TAG_ORDER}; do
    THIS="$(file_for "${t}")"
  done
fi
[ -f "${THIS}" ] || THIS="$(file_for "$(printf '%s' "${TAG_ORDER}" | tail -n1)")"

# "prev" = the release immediately before "this" in date order.
PREV=""
for t in ${TAG_ORDER}; do
  [ "${t}" = "$(basename "${THIS%%.json}")" ] && break
  PREV="$(file_for "${t}")"
done

THIS_TAG="$(jq -r '.tag' "${THIS}")"
THIS_DATE="$(jq -r '.date' "${THIS}")"
THIS_COMMIT="$(jq -r '.commit' "${THIS}")"
PREV_TAG=""
[ -n "${PREV}" ] && [ -f "${PREV}" ] && PREV_TAG="$(jq -r '.tag' "${PREV}" 2>/dev/null || true)"

metric() { # file player/sample metric -> value
  jq -r --arg k "$2" --arg m "$3" '.runs[$k][$m] // 0' "$1" 2>/dev/null || echo 0
}

FLAC="gtm/bench.flac" # canonical trend series

num() { # format an integer with thousands separators
  local n="${1:-0}"
  printf "%'d" "${n}" 2>/dev/null || echo "${n}"
}

nice_max() { # <values...> -> round y-axis ceiling
  local m=0 v
  for v in "$@"; do
    [ "${v}" -gt "${m}" ] && m="${v}"
  done
  [ "${m}" -gt 0 ] || m=1
  local mag
  mag="$(awk -v m="${m}" 'BEGIN{ x=m; k=1; while (x>=10) { x/=10; k*=10 } print k }')"
  echo $(( (m + mag - 1) / mag * mag ))
}

# ── 2. delta table ────────────────────────────────────────────────────────────
TABLE=""
for m in peak_rss_kb mean_rss_kb cpu_ms rss_5s_kb; do
  this_v="$(metric "${THIS}" "${FLAC}" "${m}")"
  prev_v=0
  [ -n "${PREV_TAG}" ] && prev_v="$(metric "${PREV}" "${FLAC}" "${m}")"
  [ "${prev_v}" -eq 0 ] 2>/dev/null && [ "${this_v}" -eq 0 ] 2>/dev/null && continue
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
  TABLE+=$'\n'"| bench.flac (gtm) | ${label} | $(num "${prev_v}") | $(num "${this_v}") | ${delta} |"
done

# ── 3. mermaid chart series (all releases, gtm/bench.flac) ────────────────────
PEAK_LABELS=""; PEAK_PTS=""
MEAN_LABELS=""; MEAN_PTS=""
LAT_LABELS="";  LAT_PTS=""
PEAK_VALUES=(); MEAN_VALUES=(); LAT_VALUES=()
for t in ${TAG_ORDER}; do
  f="$(file_for "${t}")"
  [ -f "${f}" ] || continue
  peak="$(metric "${f}" "${FLAC}" peak_rss_kb)"
  mean="$(metric "${f}" "${FLAC}" mean_rss_kb)"
  lat="$(metric "${f}" "${FLAC}" t_ready_ms)"
  PEAK_LABELS="${PEAK_LABELS} \"${t}\""
  PEAK_PTS="${PEAK_PTS} ${peak}"
  MEAN_LABELS="${MEAN_LABELS} \"${t}\""
  MEAN_PTS="${MEAN_PTS} ${mean}"
  LAT_LABELS="${LAT_LABELS} \"${t}\""
  LAT_PTS="${LAT_PTS} ${lat}"
  PEAK_VALUES+=("${peak}"); MEAN_VALUES+=("${mean}"); LAT_VALUES+=("${lat}")
done
slot() { printf '%s' "$1" | xargs | sed 's/ /, /g'; }
csv() { printf '%s' "$1" | xargs | tr ' ' ','; }
PEAK_LABELS="$(slot "${PEAK_LABELS}")"; PEAK_PTS="$(csv "${PEAK_PTS}")"
MEAN_LABELS="$(slot "${MEAN_LABELS}")"; MEAN_PTS="$(csv "${MEAN_PTS}")"
LAT_LABELS="$(slot "${LAT_LABELS}")";  LAT_PTS="$(csv "${LAT_PTS}")"
PEAK_Y="$(nice_max "${PEAK_VALUES[@]}")"
MEAN_Y="$(nice_max "${MEAN_VALUES[@]}")"
LAT_Y="$(nice_max "${LAT_VALUES[@]}")"

# ── 4. history index table ────────────────────────────────────────────────────
IDX=""
for t in ${TAG_ORDER}; do
  f="$(file_for "${t}")"
  [ -f "${f}" ] || continue
  d="$(jq -r '.date' "${f}" | cut -c1-10)"
  gpeak="$(metric "${f}" "${FLAC}" peak_rss_kb)"
  gmean="$(metric "${f}" "${FLAC}" mean_rss_kb)"
  glat="$(metric "${f}" "${FLAC}" t_ready_ms)"
  IDX+=$'\n'"| ${t} | ${d} | $(num "${gpeak}") | $(num "${gmean}") | $(num "${glat}") |"
done

# ── 5. machine-readable markers (regenerated store) ───────────────────────────
MARKERS=""
for t in ${TAG_ORDER}; do
  f="$(file_for "${t}")"
  [ -f "${f}" ] || continue
  MARKERS+=$'\n'"<!--bench:$(jq -c . "${f}")-->"
done
MARKERS="${MARKERS:1}"

THIS_TAG_SAFE="${THIS_TAG//|/}"
THIS_COMMIT_SAFE="${THIS_COMMIT//|/}"
THIS_DATE_SAFE="${THIS_DATE//|/}"
PREV_TAG_SAFE="${PREV_TAG//|/}"

if [ -z "${PREV_TAG}" ]; then
  baseline_note="No previous release results yet — the first benchmark establishes the baseline."
  table_prev_col="Prev"
  headline_suffix="(none yet)"
else
  baseline_note=""
  table_prev_col="Prev (\`${PREV_TAG_SAFE}\`)"
  headline_suffix="(\`${PREV_TAG_SAFE}\`)"
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

## Peak RSS by release (gtm, FLAC; kB)

```mermaid
xychart-beta
  title "Peak RSS by release (gtm, FLAC; kB)"
  x-axis [__PEAK_LABELS__]
  y-axis "peak RSS (kB)" 0 --> __PEAK_Y__
  bar [__PEAK_PTS__]
```

## Mean RSS trend across releases (gtm, FLAC; kB)

```mermaid
xychart-beta
  title "Mean RSS trend across releases (gtm, FLAC; kB)"
  x-axis [__MEAN_LABELS__]
  y-axis "mean RSS (kB)" 0 --> __MEAN_Y__
  line [__MEAN_PTS__]
```

## Start latency (t_ready) by release (gtm, FLAC; ms)

```mermaid
xychart-beta
  title "Start latency (t_ready) by release (gtm, FLAC; ms)"
  x-axis [__LAT_LABELS__]
  y-axis "t_ready (ms)" 0 --> __LAT_Y__
  line [__LAT_PTS__]
```

## History

| Release | Date | gtm peak RSS (kB) | gtm mean RSS (kB) | gtm t_ready (ms) |
|---------|------|-----:|-----:|-----:|__IDX__

## Machine-readable history

Past results are stored only in the markers below; `render.sh`
reconstructs trend history from them. Do not edit by hand.

__MARKERS__

## Methodology

- A single representative FLAC and one MP3 (both committed under a sealed
  fixture hash — see `assets/fixtures/` and `gen-fixtures.sh -c`) are played
  for the same window for gtm and cliamp.
- gtm runs through its headless daemon in `--test-mode` (NullMixer, no audio
  device) so CPU/RSS are measured without an audio sink.
- Metrics: peak / mean / at-5s RSS (kB, from `/proc/<pid>/status` `VmRSS`),
  CPU (ms, from `/proc/<pid>/stat` utime+stime), and t_ready (ms, IPC round
  trip to first playing state).
- Harness: `scripts/bench/run.sh <player> <file> <seconds>`; collection:
  `scripts/bench/collect.sh <tag>` writes ephemeral results to `.bench/`; this
  renderer: `scripts/bench/render.sh` publishes them into this file.
EOF
)"

PYF="$(mktemp --suffix=.py)"
cat > "${PYF}" <<'PY'
import sys
path, out = sys.argv[1], sys.argv[2]
a = sys.argv[3:]
repl = {
    "__TABLE__": a[0],
    "__PEAK_LABELS__": a[1], "__PEAK_PTS__": a[2], "__PEAK_Y__": a[3],
    "__MEAN_LABELS__": a[4], "__MEAN_PTS__": a[5], "__MEAN_Y__": a[6],
    "__LAT_LABELS__": a[7], "__LAT_PTS__": a[8], "__LAT_Y__": a[9],
    "__IDX__": a[10],
    "__THIS_TAG__": a[11], "__THIS_COMMIT__": a[12], "__THIS_DATE__": a[13],
    "__BASELINE_NOTE__": a[14], "__TABLE_PREV_COL__": a[15],
    "__HEADLINE_SUFFIX__": a[16], "__MARKERS__": a[17],
}
d = open(path).read()
for k, v in repl.items():
    d = d.replace(k, v)
open(out, "w").write(d)
PY
DOC_TMP="$(mktemp)"
printf '%s\n' "${doc}" > "${DOC_TMP}"
python3 "${PYF}" "${DOC_TMP}" "${OUT}" \
  "${TABLE}" "${PEAK_LABELS}" "${PEAK_PTS}" "${PEAK_Y}" \
  "${MEAN_LABELS}" "${MEAN_PTS}" "${MEAN_Y}" \
  "${LAT_LABELS}" "${LAT_PTS}" "${LAT_Y}" \
  "${IDX}" "${THIS_TAG_SAFE}" "${THIS_COMMIT_SAFE}" "${THIS_DATE_SAFE}" \
  "${baseline_note}" "${table_prev_col}" "${headline_suffix}" "${MARKERS}"

echo "wrote ${OUT}"