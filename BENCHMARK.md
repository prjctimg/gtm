# gtm Benchmarks

Automated measurements of gtm's resource usage during playback, compared
release-over-release and against the reference CLI player **cliamp**
([bjarneo/cliamp](https://github.com/bjarneo/cliamp)).

> **This run**: release `0.2.83+cf44858+nightly` (commit `cf44858c93d96f5b91a11c96226e05cd98ec97af`, 2026-09-10T09:49:45Z).

## Headline — diff vs the last published release (`0.2.83+3364103+nightly`)

| Fixture (player) | Metric | Prev (`0.2.83+3364103+nightly`) | This | Δ |
|------------------|--------|------:|-----:|---:|
| bench.flac (gtm) | peak RSS (kB) | 19860 | 20640 | **+780** |
| bench.flac (gtm) | mean RSS (kB) | 19817 | 20597 | **+780** |
| bench.flac (gtm) | CPU (ms) | 120 | 100 | **−20** |
| bench.flac (gtm) | RSS @5s (kB) | 19732 | 20512 | **+780** |
| bench.mp3 (gtm) | peak RSS (kB) | 19536 | 20776 | **+1240** |
| bench.mp3 (gtm) | mean RSS (kB) | 19518 | 20758 | **+1240** |
| bench.mp3 (gtm) | CPU (ms) | 240 | 180 | **−60** |
| bench.mp3 (gtm) | RSS @5s (kB) | 19428 | 20668 | **+1240** |



## Peak RSS by release (gtm vs cliamp, kB)

```mermaid
xychart-beta
  title "Peak RSS by release (gtm vs cliamp, kB)"
  x-axis ["prev", "this"]
  y-axis "peak RSS (kB)" 0 --> 120000
  bar [19860, 20640]
  bar [19536, 20776]
```

## Mean RSS trend across releases (gtm, kB)

```mermaid
xychart-beta
  title "Mean RSS trend across releases (gtm, kB)"
  x-axis [0.2.83+3364103+nightly, 0.2.83+581b9eb+nightly, 0.2.83+cf44858+nightly]
  y-axis "mean RSS (kB)" 0 --> 60000
  line [19817,20097,20597]
```
## Latency trend across releases (p50 / p95, ms)

```mermaid
xychart-beta
  title "Latency trend (p50 / p95, ms)"
  x-axis [0.2.83+cf44858+nightly, 0.2.83+3364103+nightly, 0.2.83+581b9eb+nightly, 0.2.83+7b6746d+nightly, 0.2.83+cfe43fb+nightly, 0.2.83+0ba44c5+nightly]
  y-axis "latency (ms)" 0 --> 30000
  line [20640,0,0,0,0,0]
  line [20640,0,0,0,0,0]
```


## History

| Release | Date | gtm peak RSS (kB) (FLAC) |
|---------|------|-----:|
| 0.2.83+cf44858+nightly | 2026-09-10 | 20640 |
| 0.2.83+3364103+nightly | 2026-09-08 | 19860 |
| 0.2.83+581b9eb+nightly | 2026-09-09 | 20140 |
| 0.2.83+7b6746d+nightly | 2026-09-10 | 20036 |
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
