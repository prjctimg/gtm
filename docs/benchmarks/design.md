# Benchmark design: gtm vs cliamp

This is the approved design report for automating memory/playback benchmarks
against cliamp and diffing results release-over-release in `BENCHMARK.md`.
It was written after researching what cliamp is and how to drive it headlessly.

## Reference player: cliamp

- Project: **cliamp** ("Winamp for your shell") — https://github.com/bjarneo/cliamp
- Stack: Go, Bubbletea TUI, **Beep** audio engine over **ALSA** (`libasound`);
  `yt-dlp` for YouTube; go-librespot for Spotify.
- Not Rust, and distinct from `termusic`/`cmus`.
- Format support: MP3/WAV/FLAC/OGG built-in; needs **ffmpeg** for
  AAC/ALAC/Opus/WMA.

### Headless mode (for measurement)

cliamp ships `--daemon`/`-d` which drops the TUI and visualizer — ideal for
isolating the audio engine. `--auto-play` starts playback immediately.

```bash
cliamp --daemon --auto-play --low-power --sample-rate 44100 --buffer-ms 500 Music/ &
sleep 2
# measure
grep VmRSS /proc/$PID/status
```

Flags of interest: `--low-power` (disables visualizer), `--sample-rate`,
`--buffer-ms`, `--resample-quality`, `--simplified`, `--log-level debug`.

### Measurement caveats

- cliamp exposes **no built-in RSS/CPU counters**; `cliamp status --json` gives
  playback state only. Memory must be read with OS tools:
  `pidstat`, `/proc/<pid>/status` `VmRSS`, `getrusage`, or PeakWorkingSet
  equivalents.
- Only **one daemon per user** (a single Unix socket
  `~/.config/cliamp/cliamp.sock`), so cliamp and gtm runs must be serialized.
- Daemon mode loads no Lua plugins and disables gapless preload, so its daemon
  RSS is the lower bound — the right comparison target for gtm's `gtmd`.
- The ALSA backend is dynamically linked; keep the benchmark runner focused on
  RSS/CPU, not codec-library ink.

## gtm counterpart

- gtm's daemon is `gtmd` (rodio + symphonia audio), the TUI is `gtm`.
- For parity we benchmark **gtmd** headless against cliamp's `--daemon`, both
  told to play the same fixture and then stopped after a fixed window.
- Both are Rust; RSS differs structurally (Tokio runtime, per-track decode
  buffers) so absolute numbers differ — the chart's value is the **trend and the
  delta-vs-last-release**, not a fixed ranking.

## Workloads (fixed fixtures)

Two sealed fixtures, hashes recorded in results so a corrupted fixture fails the
run loudly:

| Fixture | Purpose |
|---------|---------|
| `fixtures/sample.flac` | lossless decode path |
| `fixtures/sample.mp3` | lossy decode path |

Each played for 30 s, sampled every 100 ms, then SIGTERM.

## Metrics

| Key | Meaning | Source |
|-----|---------|--------|
| `peak_rss_kb` | max sampled RSS | `/proc/<pid>/status` `VmRSS` max |
| `mean_rss_kb` | averaged RSS over window | same series, mean |
| `rss_5s_kb` | RSS at t=5 s | series value at t=5 s |
| `cpu_ms` | CPU time (user+sys) | `/proc/<pid>/stat` utime+stime diff |
| `t_ready_ms` | time to `playing` state | player IPC `status` poll (informational) |

## Harness plan

`scripts/bench/run-bench.sh <player> <file> <seconds>` emits one JSON line per
run; `scripts/bench/render.py` turns a results JSON into `BENCHMARK.md` with
mermaid bar/line charts and a delta table vs the previous tagged results.

## Automation ("update on every release")

- CI job `benchmark` on `ubuntu-latest`, pinned to current official action
  majors (`actions/checkout@v7`, `actions/upload-artifact@v7`,
  `actions/download-artifact@v8`).
- Job runs after a release is tagged, downloads/reads the last release's
  `bench-results/<prev-tag>.json`, runs the harness, writes
  `bench-results/<tag>.json` and re-renders `BENCHMARK.md`.
- The updated `BENCHMARK.md` is committed (or folded into the release-notes PR
  for maintainer review).
- Results keyed by release tag are the source of truth; `BENCHMARK.md` charts
  the last N to show the moving trend and the vs-last diff.

## Mermaid sketches

Bar chart (gtm vs cliamp, per fixture) and line chart (peak RSS across
releases). See `BENCHMARK.md` for concrete renderable snippets.

## Next steps (when approved)

1. Add `scripts/bench/` harness + fixtures.
2. Add the `benchmark` CI job to `release.yml`.
3. Wire renderer + commit step.
4. Track a baseline run to seed `bench-results/`.