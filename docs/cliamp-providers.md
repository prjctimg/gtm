# Plan: provider support in cliamp

This is the approved design *plan* for how a cliamp-style player ("Winamp for
your shell") could expose the same streaming-provider surface gtm already
ships. It is intentionally a plan, not an implementation. cliamp is used as the
benchmark reference player (see `docs/benchmarks/design.md`); this document
spells out what providers mean for it and how the work would be phased.

## Backdrop

cliamp (https://github.com/bjarneo/cliamp) is a Go + Bubbletea terminal player
with a Beep/ALSA audio core and optional `yt-dlp`/go-librespot integrations. It
plays local files first-class; everything else is bolted on per-service.
gtm's daemon (`gtmd`) already centralises the same problem: one IPC surface
(`gtmd.1`), one queue, one scrobble pipeline, and a set of providers behind a
uniform `provider://` path scheme. This plan is about giving cliamp that same
provider discipline without re-implementing audio.

## Design principle

Keep cliamp's audio engine the way it is. Add an **abstract provider layer**
between the player core and the network, exactly like gtm's `resolve_remote` /
`play_remote` split:

```
player core ──▶ provider://writer ──▶ remote loader ──▶ stream bytes ──▶ audio core
                       │
                       └──▶ metadata / now-playing / scrobble
```

## Provider inventory (mirrors gtm)

| Provider | gtm path prefix | Offered by cliamp plan |
|----------|-----------------|------------------------|
| Radio Browser (radio-browser.info) | `radio://` | browse/search stations by tag/country, custom stations file, ICEcast/Shoutcast `StreamTitle` |
| Subsonic / Navidrome | `subsonic://` | server creds, index search, stream a track id |
| Podcasts | `podcast://` | RSS/Atom subscribe, episode list, feed refresh, stream by feed+index |
| Spotify | `spotify://track:…` | PKCE browser/OAuth token, library + playlists via Web API, stream via librespot |
| Last.fm | (scrobble only) | now-playing + scrobble threshold shared across every source |
| Plain HTTP(S) stream | `stream://` | arbitrary URL, reused by all the others |

Every provider resolves to a URL plus a display identity; the player core never
sees provider internals.

## Phased work

### Phase 1 — provider core

- Introduce `provider://` path parsing in the player (mirror
  `gtm-core`'s `parse_remote_path`): a URI with a kind + provider-specific id.
- Add the `radio://` kind first (cheapest, most self-contained):
  - radio-browser.org search/top/tags/country API calls.
  - a `radios.toml` custom-station store (same location semantics as gtm:
    `$XDG_CONFIG_HOME/<app>/radios.toml`).
  - ICEcast/Shoutcast metadata stripping with `StreamTitle` capture.
- Route resolved bytes into the existing audio core. Live streams must never
  block the audio callback: feed through a buffering reader (gtm uses a 6 s
  ring-buffer decode thread — see `gtm-audio/src/buffer.rs` and the reader
  decode path in `gtmd/src/daemon.rs`).

### Phase 2 — on-demand providers

- `subsonic://` and `podcast://`: configure creds/feeds, list + search,
  stream episode/track ids. Both are seekable on-demand sources; reuse the
  local pipeline for seeks via a re-opening HTTP reader.
- Streaming HTTP `stream://` for raw URLs/M3U/PLS playlists.

### Phase 3 — account providers + scrobble

- Spotify PKCE OAuth with a loopback callback server bound *before* the
  browser opens (the gtm fix sequence is the template), keychain token
  storage, playlist sync, librespot/WASAPI-passthrough playback.
- Last.fm session via the same OAuth helper; now-playing updates and the
  scrobble threshold on top of unified track identity.

## Shared abstractions (required before Phase 3)

- **Unified track identity**: every played thing has `(provider, id, title,
  artist, album, duration)` so queue, history, and scrobble all speak one
  struct. gtm models this as `TrackInfo` with the provider implied by the
  path.
- **Credential store**: one secure keychain entry per provider, never logged,
  never committed.
- **Browser OAuth helper**: shared, opens browser, binds loopback listener
  eagerly, falls back to paste-on-stdin for headless.
- **Reconnect policy**: `live` vs on-demand is a per-kind flag that decides
  whether seeks reconnect the transport or are dropped.

## Not in scope

- Decoder/codec work (local formats stay as-is; providers hand bytes to the
  core).
- Visualizer/equalizer beyond what the audio core already exposes.
- Any change to the benchmark harness that drives cliamp headlessly — `--daemon`
  RSS/CPU measurements must remain unaffected.

## Open questions

1. Provider metadata surfaced in the TUI: one combined "Sources" list per
   provider with per-provider screens, or a single search across providers?
2. M3U/PLS playlist handling server- or client-side for `stream://`.
3. Whether to adopt gtm's exact `radio://`/`subsonic://` path scheme (max
   parity, easy migration) or define cliamp-specific prefixes.