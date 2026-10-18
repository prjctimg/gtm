# Playback source on the wire

Status: **implemented**. Follows `REACTIVE_TOKENS.md`, which moved colour derivation
to gtmd. This adds the one fact a client needs to label where audio came from.

Two things landed differently from how this reads, both because the frame is
internally tagged JSON (`{"event": ..., ...}`) rather than the positional
StructTuple assumed in §1, and because `radio`, `podcast` and `chart` cannot all
be recovered from a path alone. See "What changed" at the bottom.

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

The bundled TUI reads it: a dim glyph on the artist line of the now-playing
panel and of the zen header, for every source except `local`. `gtm.nvim` maps
the value to a marker:

```lua
local SRC_ICON = { local = "♫", spotify = "◉", radio = "☁", podcast = "❋", chart = "▤" }
```

Unknown values render nothing rather than guessing.

## What changed

**The frame is internally tagged, not positional.** `playback_started` is
`#[serde(tag = "event")]` on `DaemonEvent`, so the wire form is an object and
every key is named. That makes the compatibility argument stronger rather than
weaker — an old client ignores the extra key outright — so `source` stayed
trailing anyway, which keeps it in the same position for any client that reads
the object positionally by mistake.

**`chart` needed a route to be reachable.** `radio://`, `podcast://` and
`spotify:` are all distinguishable from the path. A bare `https://` is not: an
Apple chart preview, a Deezer preview and a station the user pasted by hand are
the same shape of URL, resolving to the same bytes, with no length. Reporting
all three as `radio` would have made the field half-wrong.

So charts now emit `chart://<url>`, parsed by `parse_remote_path` into a new
`RemoteKind::Chart` that fetches exactly like `RemoteKind::Stream` and is `live`
in the same way. The prefix is the whole mechanism, and it is written once in
`stream.rs` — both `chart_uri()`, which the providers call, and the parse arm,
which lives beside it — so the two sides cannot drift.

The classification itself is `source_for_path`, one function over the path, used
by every site that can start playback: a queued row, a resumed position and a
crossfaded promotion all re-derive it from the same string rather than carrying a
value forward from whatever happened to be playing.

**`yt-dlp` resolves to `local`.** The bytes are remote; the playback is a file
the mixer decodes start to finish, and `local` is the marker that means "this
ends".

`local` is deliberately unmarked in the bundled client. A glyph on every song
is a permanent decoration saying nothing; the information a listener wants is
that audio is arriving from somewhere the library does not account for. A
client that disagrees is free to mark all five.

## Not done

The client maps the source to a glyph and nothing more. It does not use the
value for behaviour — no routing, no scrobble policy, no per-source EQ — so a
source arriving from the daemon changes what is drawn and nothing else.