# gtm Benchmarks

Automated measurements of gtm's resource usage during playback, compared
release-over-release and against the reference CLI player **cliamp**
([bjarneo/cliamp](https://github.com/bjarneo/cliamp)).

> **This run**: release `0.2.83+3364103+nightly` (commit `336410342811a809587a27820b9d4fcc355237a5`, 2026-09-08T20:00:21Z).

## Headline — diff vs the last published release (`0.2.83+cfe43fb+nightly`)

| Fixture (player) | Metric | Prev (`0.2.83+cfe43fb+nightly`) | This | Δ |
|------------------|--------|------:|-----:|---:|
| bench.flac (gtm) | peak RSS (kB) | 19492 | 19860 | **+368** |
| bench.flac (gtm) | mean RSS (kB) | 19449 | 19817 | **+368** |
| bench.flac (gtm) | CPU (ms) | 120 | 120 | 0 |
| bench.flac (gtm) | RSS @5s (kB) | 19364 | 19732 | **+368** |
| bench.mp3 (gtm) | peak RSS (kB) | 19924 | 19536 | **−388** |
| bench.mp3 (gtm) | mean RSS (kB) | 19905 | 19518 | **−387** |
| bench.mp3 (gtm) | CPU (ms) | 210 | 240 | **+30** |
| bench.mp3 (gtm) | RSS @5s (kB) | 19816 | 19428 | **−388** |



## Peak RSS by release (gtm vs cliamp, kB)

```mermaid
xychart-beta
  title "Peak RSS by release (gtm vs cliamp, kB)"
  x-axis ["prev", "this"]
  y-axis "peak RSS (kB)" 0 --> 120000
  bar [19492, 19860]
  bar [19924, 19536]
```

## Mean RSS trend across releases (gtm, kB)

```mermaid
xychart-beta
  title "Mean RSS trend across releases (gtm, kB)"
  x-axis [0.2.83+cfe43fb+nightly, 0.2.83+3364103+nightly]
  y-axis "mean RSS (kB)" 0 --> 60000
  line [19449,19817]
```

## History

| Release | Date | gtm peak RSS (kB) (FLAC) |
|---------|------|-----:|
| 0.2.83+3364103+nightly | 2026-09-08 | 19860 |
| 0.2.83+cfe43fb+nightly | 2026-09-08 | 19492 |

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
