# gtm Benchmarks

Automated measurements of gtm's resource usage during playback, compared
release-over-release and against the reference CLI player **cliamp**
([bjarneo/cliamp](https://github.com/bjarneo/cliamp)).

> **This run**: release `0.2.83+7b6746d+nightly` (commit `7b6746d893eb83d1de2e18e9289c416420bb7b94`, 2026-09-10T06:06:28Z).

## Headline — diff vs the last published release (`0.2.83+3364103+nightly`)

| Fixture (player) | Metric | Prev (`0.2.83+3364103+nightly`) | This | Δ |
|------------------|--------|------:|-----:|---:|
| bench.flac (gtm) | peak RSS (kB) | 19860 | 20036 | **+176** |
| bench.flac (gtm) | mean RSS (kB) | 19817 | 19993 | **+176** |
| bench.flac (gtm) | CPU (ms) | 120 | 170 | **+50** |
| bench.flac (gtm) | RSS @5s (kB) | 19732 | 19908 | **+176** |
| bench.mp3 (gtm) | peak RSS (kB) | 19536 | 20856 | **+1320** |
| bench.mp3 (gtm) | mean RSS (kB) | 19518 | 20838 | **+1320** |
| bench.mp3 (gtm) | CPU (ms) | 240 | 290 | **+50** |
| bench.mp3 (gtm) | RSS @5s (kB) | 19428 | 20748 | **+1320** |



## Peak RSS by release (gtm vs cliamp, kB)

```mermaid
xychart-beta
  title "Peak RSS by release (gtm vs cliamp, kB)"
  x-axis ["prev", "this"]
  y-axis "peak RSS (kB)" 0 --> 120000
  bar [19860, 20036]
  bar [19536, 20856]
```

## Mean RSS trend across releases (gtm, kB)

```mermaid
xychart-beta
  title "Mean RSS trend across releases (gtm, kB)"
  x-axis [0.2.83+3364103+nightly, 0.2.83+581b9eb+nightly, 0.2.83+7b6746d+nightly]
  y-axis "mean RSS (kB)" 0 --> 60000
  line [19817,20097,19993]
```

## History

| Release | Date | gtm peak RSS (kB) (FLAC) |
|---------|------|-----:|
| 0.2.83+7b6746d+nightly | 2026-09-10 | 20036 |
| 0.2.83+3364103+nightly | 2026-09-08 | 19860 |
| 0.2.83+581b9eb+nightly | 2026-09-09 | 20140 |
| 0.2.83+cfe43fb+nightly | 2026-09-08 | 19492 |
| 0.2.83+0ba44c5+nightly | 2026-09-09 | 19512 |

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
