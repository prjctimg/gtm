#!/usr/bin/env bash
# gtm benchmark harness — measures gtm/gtmd and cliamp resource usage during
# playback and emits a JSON result line. See BENCHMARK.md for methodology.
#
#   scripts/bench/run-bench.sh <player> <file> <seconds>
#
#   player   gtm | cliamp
#   file     path to an audio file (fixture under bench-results/fixtures)
#   seconds  sampling window length
#
# Emits to stdout:
#   {"player":..,"file":..,"file_sha256":..,"peak_rss_kb":..,"mean_rss_kb":..,
#    "rss_5s_kb":..,"cpu_ms":..,"t_ready_ms":..,"error":..}
#
# gtm is measured through its headless daemon (gtmd --test-mode => NullMixer,
# no real audio device, so the harness records CPU/RSS without an audio sink
# under CI). cliamp runs its own --daemon mode. Both are serialized because
# cliamp allows only a single instance per user.

set -euo pipefail

PLAYER="${1:?usage: run-bench.sh <player> <file> <seconds>}"
FILE="${2:?usage: run-bench.sh <player> <file> <seconds>}"
SECONDS="${3:?usage: run-bench.sh <player> <file> <seconds>}"

case "${SECONDS}" in
  '' | *[!0-9]*) echo "seconds must be a positive integer" >&2; exit 2 ;;
esac
[ "${SECONDS}" -gt 0 ] || { echo "seconds must be a positive integer" >&2; exit 2; }
[ -f "${FILE}" ] || { echo "file not found: ${FILE}" >&2; exit 2; }

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GTM="${GTM_BIN:-${REPO_DIR}/target/release/gtm}"
GTMD="${GTMD_BIN:-${REPO_DIR}/target/release/gtmd}"
[ -x "${GTM}" ] || GTM="${GTM_BIN:-$(command -v gtm || true)}"
[ -x "${GTMD}" ] || GTMD="${GTMD_BIN:-$(command -v gtmd || true)}"
CLIAMP="${CLIAMP_BIN:-$(command -v cliamp || true)}"

SOCK_DIR="$(mktemp -d)"
SOCK="${SOCK_DIR}/gtmd.sock"
PID=""
t_ready_ms=0
RESULT_JSON=""
# Linux clock ticks per second: /proc/<pid>/stat utime/stime are in these units.
CLK_TCK="$(getconf CLK_TCK 2>/dev/null || echo 100)"

cleanup() {
  [ -n "${PID}" ] && kill "${PID}" 2>/dev/null || true
  [ -n "${PID}" ] && wait "${PID}" 2>/dev/null || true
  PID=""
  rm -rf "${SOCK_DIR}"
}
trap cleanup EXIT

# Sample RSS (kB) and CPU (ms) for a process every ~100ms for `ms` ms,
# printing one "ms rss cpu" line per tick to the given file.
sample_proc() {
  local pid="$1" ms="$2" out="$3"
  local end=$(($(date +%s%N) + ms * 1000000))
  local t=0 rss utime stime
  while kill -0 "${pid}" 2>/dev/null && [ "$(date +%s%N)" -lt "${end}" ]; do
    if [ -r "/proc/${pid}/status" ]; then
      rss="$(awk '/^VmRSS:/ {print $2; exit}' "/proc/${pid}/status")"
    else
      rss=0
    fi
    if [ -r "/proc/${pid}/stat" ]; then
      read -r utime stime < <(awk '
        { s=index($0, ") ")+2; rest=substr($0, s); n=split(rest, a, " ");
          # after comm field: state, ppid, pgrp, session, tty_nr, tpgid, flags,
          # minflt, cminflt, majflt, cmajflt, utime, stime  => a[12], a[13]
          print a[12], a[13] }' "/proc/${pid}/stat")
    else
      utime=0; stime=0
    fi
    local cpu_ms=$(( (${utime:-0} + ${stime:-0}) * (1000 / CLK_TCK) ))
    printf '%s %s %s\n' "${t}" "${rss:-0}" "${cpu_ms}" >> "${out}"
    t=$((t + 100))
    # slice the sleep so a short window still gets ~10 ticks/sec
    local left=100
    while [ "${left}" -gt 0 ]; do
      /bin/sleep 0.02
      left=$((left - 20))
    done
  done
}

# Aggregate the sampled lines into computed fields, writing the four vars
# (peak_rss, mean_rss, rss_5s, peak_cpu) back into the caller's scope via eval.
aggregate() {
  local file="$1"
  local peak=0 mean=0 rss5=0 cpu=0 n=0 sum=0 best_rss=0 best_dist=999999
  local -a sorted_values=()
  while read -r t rss c; do
    n=$((n + 1))
    sum=$((sum + rss))
    [ "${rss}" -gt "${peak}" ] && peak="${rss}"
    [ "${c}" -gt "${cpu}" ] && cpu="${c}"
    sorted_values+=("${rss}")
    # rss at ~5s: track the sample closest to t=5000ms
    local d=$(( (t>5000?t-5000:5000-t) ))
    if [ "${d}" -lt "${best_dist}" ]; then best_dist="${d}"; best_rss="${rss}"; fi
  done < "${file}"
  [ "${n}" -gt 0 ] && mean=$(( sum / n ))
  rss5="${best_rss:-0}"
  local p50=0 p95=0
  if [ "${#sorted_values[@]}" -gt 0 ]; then
    IFS=$'\n' sorted_values=($(printf '%s\n' "${sorted_values[@]}" | sort -n)); unset IFS
    local count=${#sorted_values[@]}
    local p50_idx=$(( (count - 1) / 2 ))
    local p95_idx=$(( count * 95 / 100 ))
    [ "${p95_idx}" -ge "${count}" ] && p95_idx=$(( count - 1 ))
    p50="${sorted_values[$p50_idx]}"
    p95="${sorted_values[$p95_idx]}"
  fi
  eval "PEAK_RSS=${peak}; MEAN_RSS=${mean}; RSS_5S=${rss5}; CPU_MS=${cpu}; P50_LATENCY=${p50}; P95_LATENCY=${p95}"
}

wait_socket() {
  local path="$1" tries=300
  while [ "${tries}" -gt 0 ]; do
    [ -S "${path}" ] && return 0
    /bin/sleep 0.05
    tries=$((tries - 1))
  done
  return 1
}

run_gtm() {
  "${GTMD}" --test-mode --socket "${SOCK}" --library "${SOCK_DIR}/lib.db" \
    --config "${SOCK_DIR}/cfg" >"${SOCK_DIR}/gtmd.log" 2>&1 &
  PID=$!
  if ! wait_socket "${SOCK}"; then
    RESULT_JSON="$(emit_error "gtmd did not create its socket")"
    return 1
  fi
  local t0 t1
  t0="$(date +%s%N)"
  "${GTM}" --socket "${SOCK}" play "${FILE}" >/dev/null 2>&1 || {
    RESULT_JSON="$(emit_error "gtm play failed")"; return 1; }
  t1="$(date +%s%N)"
  t_ready_ms=$(( (t1 - t0) / 1000000 ))

  sample_proc "${PID}" $((SECONDS * 1000)) "${SOCK_DIR}/samples" || true
  kill "${PID}" 2>/dev/null || true
  wait "${PID}" 2>/dev/null || true
  PID=""
}

run_cliamp() {
  [ -n "${CLIAMP}" ] || {
    RESULT_JSON="$(emit_error "cliamp not found; install it or set CLIAMP_BIN")"; return 1; }
  # cliamp's socket is '$HOME/.config/cliamp/cliamp.sock'; isolate HOME so a
  # user's running instance (and its config) is never touched.
  export HOME="${SOCK_DIR}/home"
  mkdir -p "${HOME}"
  # --low-power reduces CPU (lower UI cadence / no visualizer) in the daemon;
  # the explicit sample-rate/buffer keep PCM as close to the gtm measurement
  # as possible.
  "${CLIAMP}" --daemon --low-power --sample-rate 44100 --buffer-ms 500 \
    >"${SOCK_DIR}/cliamp.log" 2>&1 &
  PID=$!
  local tries=300
  local sockpath=""
  while [ "${tries}" -gt 0 ]; do
    sockpath="$(find "${HOME}/.config/cliamp" -name '*.sock' -print -quit 2>/dev/null || true)"
    [ -n "${sockpath}" ] && break
    /bin/sleep 0.05
    tries=$((tries - 1))
  done
  [ -n "${sockpath}" ] || {
    RESULT_JSON="$(emit_error "cliamp did not create a socket (see cliamp.log)")"; return 1; }

  local t0 t1
  t0="$(date +%s%N)"
  "${CLIAMP}" queue "${FILE}" >/dev/null 2>&1 \
    && "${CLIAMP}" play >/dev/null 2>&1 \
    || { RESULT_JSON="$(emit_error "cliamp queue/play failed (see cliamp.log)")"; return 1; }
  t1="$(date +%s%N)"
  t_ready_ms=$(( (t1 - t0) / 1000000 ))

  sample_proc "${PID}" $((SECONDS * 1000)) "${SOCK_DIR}/samples" || true
  kill "${PID}" 2>/dev/null || true
  wait "${PID}" 2>/dev/null || true
  PID=""
  unset HOME
}

emit_error() {
  jq -nc --arg player "${PLAYER}" --arg file "${FILE}" --arg error "$1" \
    '{player:$player,file:$file,error:$error}'
}

case "${PLAYER}" in
  gtm) run_gtm ;;
  cliamp) run_cliamp ;;
  *) echo "unknown player: ${PLAYER} (expected gtm or cliamp)" >&2; exit 2 ;;
esac

if [ -n "${RESULT_JSON}" ]; then
  printf '%s\n' "${RESULT_JSON}"
  exit 0
fi

if [ -f "${SOCK_DIR}/samples" ] && [ -s "${SOCK_DIR}/samples" ]; then
  PEAK_RSS=0; MEAN_RSS=0; RSS_5S=0; CPU_MS=0; P50_LATENCY=0; P95_LATENCY=0
  aggregate "${SOCK_DIR}/samples"
  FILE_SHA="$(sha256sum "${FILE}" | awk '{print $1}')"
  jq -nc --arg player "${PLAYER}" --arg file "${FILE}" --arg sha "${FILE_SHA}" \
    --argjson peak "${PEAK_RSS}" --argjson mean "${MEAN_RSS}" \
    --argjson rss5 "${RSS_5S}" --argjson cpu "${CPU_MS}" \
    --argjson ready "${t_ready_ms}" \
    --argjson p50 "${P50_LATENCY}" --argjson p95 "${P95_LATENCY}" \
    '{player:$player,file:$file,file_sha256:$sha,peak_rss_kb:$peak,mean_rss_kb:$mean,rss_5s_kb:$rss5,cpu_ms:$cpu,t_ready_ms:$ready,p50_latency_kb:$p50,p95_latency_kb:$p95}'
else
  emit_error "no samples collected"
fi
