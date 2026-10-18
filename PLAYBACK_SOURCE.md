# Playback source on the wire

Status: **proposed**. Follows `REACTIVE_TOKENS.md`, which moved colour derivation
to gtmd. This adds the one fact a client needs to label where audio came from.

## Problem

Clients cannot tell Spotify from a radio stream from a local file. `TrackInfo`
has no source field, `PlaybackStatus` is a bare state enum, and `DaemonState`
carries none. The only `provider` string in the protocol is `SetCoverProvider`,
which selects a cover lookup backend — unrelated — and `ProviderError`, which
only fires on failure.

`gtm.nvim` wants to show a source marker in its statusline and has nothing to
key it on.

## Change

### 1. `playback_started` gains a trailing field

```rust
#[serde(rename = "playback_started")]
PlaybackStarted {
    track: Box<TrackInfo>,
    auto_advanced: bool,
    time_pos: f64,
    duration: f64,
    /// Where the audio came from. See `SRC_*` below. Empty when unknown.
    source: String,
},
```

Trailing, so the StructTuple frame is `["playback_started", track, auto, pos,
dur, source]` and every existing reader — which indexes positions it knows — is
unaffected. The only skew is a new client against an old daemon, which is the
same situation `reactive_theme` already has.

Do **not** add it to `TrackInfo`: a file's source does not change with how you
played it, and the field would have to be threaded through the library.

### 2. The status snapshot carries it too

`get_status` and `get_status_lite` answer with a decoded object. Add `source` as
a sibling of `current_track`, so a client that primes on connect has the marker
before the first `playback_started` rather than after it.

### 3. Values

Lowercase, stable, from a fixed set. Anything else is an unknown source and
clients render nothing.

| Value | Meaning |
|---|---|
| `local` | A file on disk |
| `spotify` | Spotify, via librespot |
| `radio` | A Radio Browser stream |
| `podcast` | A podcast episode |
| `chart` | A charts provider stream |

Set it where the daemon already knows which provider produced the item — the
same branch that decides which provider module handles the request. Clear it on
stop and on track end, alongside `current_track`.

## Rules

- One value, set once per track. Do not emit a separate event.
- Never widen the set without checking `gtm.spec`; clients match on the string.
- Document the field in `docs/man/gtmd-ipc.1.md` and `gtm.spec`.

## Consumer

`gtm.nvim` maps the value to a marker:

```lua
local SRC_ICON = { local = "♫", spotify = "◉", radio = "☁", podcast = "❋", chart = "▤" }
```

Unknown values render nothing rather than guessing.