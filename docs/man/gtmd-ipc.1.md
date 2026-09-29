% gtmd-ipc(1) gtmd IPC protocol manual
% prjctimg
% 2026

# NAME

gtmd-ipc - IPC protocol for the gtm music daemon

# DESCRIPTION

**gtmd**(1) listens on two Unix domain sockets. A client sends newline-delimited
JSON commands on the command socket and receives one JSON response per command,
interleaved with unsolicited JSON event objects. A second, read-only socket
carries the same events as length-prefixed MessagePack frames, for clients that
want position updates without paying for JSON parsing.

Every command is synchronous: the daemon answers before doing the work, for
anything that can be deferred. Where that is not true — a background library
scan, a YouTube download — the command returns an acknowledgement immediately and
the result arrives later as an event, and the command table below says which.
This is the single most important thing to know about the protocol, and it is
the reason a successful response is not the same as a finished one.

Commands are dispatched by the `cmd` string. Parameters are the remaining keys of
the same object: there is no `params` wrapper on the wire, even though the
internal representation has one. Command and field names are `snake_case`.

The wire version is **3** (`gtm/src/shared/ipc.rs`).

| Socket | Path | Carries |
|--------|------|---------|
| Command | `$XDG_RUNTIME_DIR/gtm/gtmd.sock` | Commands, responses, JSON events |
| Pulse | `$XDG_RUNTIME_DIR/gtm/gtmd.pulse` | MessagePack event frames, read-only |

If `$XDG_RUNTIME_DIR` is unset the daemon falls back, in order, to
`/tmp/gtm-$USER/gtm/gtmd.sock`, `$TMPDIR/gtm/gtmd.sock`, then
`$HOME/.gtm/gtm/gtmd.sock`.

# FRAMING

## Requests

One JSON object per line, newline-terminated:

```json
{"id": 1, "cmd": "play", "path": "/music/song.flac", "start_pos": 0.0}
```

`id` is a client-chosen correlation token, echoed on the response. The daemon
does not require it to be monotonic, but two in-flight requests sharing an `id`
make the response ambiguous.

## Responses

One JSON object per line, newline-terminated:

```json
{"id": 1, "ok": true, "volume": 80}
```

`ok` is always present. On failure, `error` carries a human-readable message and
no other keys. On success, the remaining keys are the command's payload — see
the tables. A command documented with no payload returns nothing beyond `ok`.

## Events (JSON, command socket)

Event objects have an `event` key where a response has `id`, which is how a
client tells them apart on a multiplexed connection:

```json
{"event": "playback_started", "track": {"title": "Song"}, "time_pos": 0.0, "duration": 240.0}
```

The bundled **gtm**(1) client does not read this stream; it consumes events only
from the pulse socket, so that each event is delivered exactly once. Third-party
clients may read either.

## Events (binary, pulse socket)

```
[4 bytes: payload length, big-endian uint32][MessagePack payload]
```

The payload is a MessagePack map carrying the same fields as the JSON form. The
two framings are told apart by the first byte: `0x7B` (`{`) is a JSON line,
anything else is a binary frame.

Limits: 1 MiB maximum JSON line, 16 MiB maximum binary frame.

# RESPONSE DECODING

Responses are not decoded by command name. `ok_from_data` inspects the **shape**
of the payload, which has consequences worth stating plainly:

- An object with a `tracks` array decodes as a track list, whatever asked for it.
- An object with a `queue` array decodes as queue state; `cursor` is read
  separately and defaults to `0` when absent.
- An object with `playlists` decodes as a playlist list.
- An object with a `running` key decodes as sync status.
- **Anything else passes through undecoded**, with the whole object handed to the
  client as an opaque value.

That last case is why several `library` actions answer with a bare count
(`removed`, `refreshed`, `deduped`) rather than a recognised structure. It is
also why a client must not assume an action it did not recognise failed: an
unknown action is a request error, but an unrecognised *response shape* is a
successful response with an unexamined payload.

# COMMAND REFERENCE
# COMMAND REFERENCE


### Playback

Every command is a control call on the current session and returns as soon as the daemon has accepted it; nothing waits for the audio device to catch up. State arrives as events (`position_changed`, `playback_started`), not as a return value, so a client that wants to confirm an outcome should watch for the event rather than re-read the response. `play` is the only one that takes a path, and it is the only one that can fail on the spot.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `play` | `path` : string, `start_pos` : float | — | `playback_started` |
| `play_stream` | `url` : string | — | `playback_started`, `radio_title_changed` |
| `play_pause` | — | — | `playback_started` / `playback_paused` |
| `pause` | — | — | `playback_paused` |
| `stop` | — | — | `playback_stopped` |
| `next` | — | — | `playback_started` |
| `prev` | — | — | `playback_started` |
| `seek` | `position_secs` : float | — | — |
| `set_volume` | `volume` : u8 | — | `volume_changed` |
| `get_volume` | — | `volume` | — |
| `toggle_shuffle` | — | — | `shuffle_changed` |
| `cycle_repeat` | `mode` : RepeatMode | — | `repeat_mode_changed` |
| `toggle_mute` | — | — | `volume_changed` |
| `set_mono` | `enabled` : bool | — | — |
| `quit` | — | — | — |
| `ping` | — | — | — |

### Queue

One `cmd` with an `action` discriminator, not one command per operation. `action` is a nested enum: sending an unknown action is a protocol error rather than a no-op, so a client should not synthesise one. `list` is the only read; every other action mutates and emits `queue_changed`.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `queue` | `action` : QueueAction | `queue`, `cursor` | `queue_changed` (every action except `list`) |

### Library

One `cmd` with an `action` discriminator, and the one group where several actions are explicitly asynchronous: `scan`, `sync_covers`, `sync_lyrics` and `sync_metadata` return immediately with no payload and finish later as events. A client must not treat a successful response as "done" for those. `sync_status` is the only way to ask what state a background pass is in.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `library` | `action` : LibraryAction | `removed` | `metadata_changed` (the three `sync_*` actions), `library_organized` (`organize`) |

### Search and favourites

Synchronous reads against the local index. `search` accepts `query` and nothing else — there is no fuzzy, field-selecting or diacritic-folding variant on the wire, and a client sending extra keys is relying on server-side defaults it cannot see. Favourites are a separate, explicitly id-addressed set.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `search` | `query` : string | `tracks` | — |
| `get_favourites` | — | `tracks` | — |
| `add_favourite` | `track_id` : i64 | — | — |
| `remove_favourite` | `track_id` : i64 | — | — |

### Cover art and lyrics

Both are cache-backed reads that may touch the network on a miss, and both can be slow enough that a client wants a timeout around them. `get_cover_art` returns base64 image bytes, not a URL — the daemon does not hand out paths. Lyrics are `null` rather than absent when nothing was found, so the field's presence is not proof of a hit.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `get_cover_art` | `track_id` : i64, `path` : string | `data` | — |
| `artist_cover_art` | `artist` : string | `data` | — |
| `get_lyrics` | `track_id` : i64, `path` : string | `lyrics` | — |
| `lyrics_search` | `artist` : string, `title` : string, `album` : string, `duration` : float | `stations` | — |
| `get_cover_cache_stat` | — | `stat`, `disk_bytes`, `mem_bytes`, `cap_bytes` | — |
| `set_cover_provider` | `provider` : string | — | — |
| `set_cover_cache` | `bytes` : uint64 | — | — |
| `clear_cache` | `what` : CacheKind | — | — |

### Audio processing

DSP configuration. Every setter is acknowledged with no payload and reports the applied value as its own event, so a client that needs confirmation subscribes rather than polls. Preset names come from `list_eq_presets`; an unknown preset is rejected at the daemon.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `set_eq_preset` | `preset` : EqPreset | — | `eq_preset_changed` |
| `set_eq_enabled` | `enabled` : bool | — | `eq_enabled_changed` |
| `list_eq_presets` | — | `presets` | — |
| `set_reverb` | `enabled` : bool, `room_size` : float | — | `reverb_changed` |
| `crossfade` | `enabled` : bool, `duration_secs` : u8 | — | `crossfade_changed` |
| `set_gapless` | `enabled` : bool | — | `gapless_changed` |
| `set_dynamic_mode` | `enabled` : bool, `min_queue_remaining` : uint32, `max_history` : uint32 | — | `dynamic_mode_changed` |
| `set_low_power` | `enabled` : bool | — | — |
| `get_low_power` | — | `low_power` | — |
| `set_pre_gain` | `pre_gain_db` : float | — | `pre_gain_changed` |
| `set_loudness_mode` | `mode` : LoudnessMode | — | `loudness_mode_changed` |
| `scan_loudness` | `track_ids` : Vec<i64>, `force` : bool | — | `loudness_scan_progress`, `loudness_scan_done` |
| `set_audio_device` | `name` : string | — | — |
| `list_audio_devices` | — | `devices` | — |

### Radio

Directory lookups against radio-browser.info, plus playback and tracklist reads. Directory calls are ordinary network reads and fail when the directory does. `radio_play` is the exception: it takes a station id and starts a non-seekable live stream. Locally stored stations are addressed as `custom:N`, 1-based into `radios.toml` — that file is owned by the `gtm` client and is not writable over IPC. `radio_tracklist` returns an empty `tracks` array when the station publishes none, which is not an error.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `radio_search` | `query` : string, `limit` : uint16 | `stations` | — |
| `radio_top` | `limit` : uint16 | `stations` | — |
| `radio_tags` | `limit` : uint16 | `tags` | — |
| `radio_bytag` | `tag` : string, `limit` : uint16 | `stations` | — |
| `radio_countries` | `limit` : uint16 | `countries` | — |
| `radio_bycountry` | `country` : string, `limit` : uint16 | `stations` | — |
| `radio_tracklist` | `station_id` : string | `list` | — |
| `radio_play` | `station_id` : string, `station_name` : string | — | `playback_started`, `radio_title_changed` |

### Spotify

Two independent legs share one stored token, and the distinction decides which client id a request is billed against. Playback and library calls run over the Web API; only playback additionally needs a Connect session. `spotify_status` is the only command whose `linked` field a client should branch on, and every other command here fails with an error rather than a null when the account is unlinked.

The client id matters: rate limits are keyed per Spotify app, not per user, so a Web API token minted under a shared app id contends with every other install using it. `spotify_oauth_start` therefore takes an optional `client_id` naming the app to authorize against; omit it and the daemon falls back to its own built-in id. That id is recorded with the token, because a refresh must be presented to the app that ran the authorization. Playback is unaffected by it — the Connect session always registers as the daemon's own app, which is the only recognised playback app.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `spotify_status` | — | `enabled`, `api_key`, `session_token`, `ready`, `loved`, `error` | — |
| `spotify_oauth_start` | `port` : uint16, `client_id` : string | `url` | — |
| `spotify_cancel_oauth` | — | — | — |
| `spotify_set_token` | `token` : string | `status` | — |
| `spotify_clear` | — | — | — |
| `spotify_playlists` | — | `playlists` | — |
| `spotify_playlist_tracks` | `id` : string | `tracks` | — |
| `spotify_web_playlist_tracks` | `uri` : string | `tracks` | — |
| `spotify_album_tracks` | `uri` : string | `tracks` | — |
| `spotify_artist_top_tracks` | `uri` : string | `tracks` | — |
| `spotify_track_image` | `image_url` : string | `data` | — |
| `spotify_search_web` | `query` : string | `tracks` | — |
| `spotify_resolve` | `playlist_id` : string, `track_index` : int, `play` : bool | — | — |
| `spotify_resolve_track` | `name` : string, `artists` : string, `album` : string, `uri` : string, `image_url` : string, `play` : bool | — | — |
| `spotify_match` | `query` : string | `uri` | — |
| `spotify_sync` | — | — | — |
| `spotify_play_pause` | — | — | `playback_started` / `playback_paused` |
| `spotify_next` | — | `status` | `playback_started` |
| `spotify_previous` | — | `status` | `playback_started` |
| `spotify_seek` | `pos_secs` : uint32 | `status` | — |
| `spotify_shuffle` | `on` : bool | `status` | — |
| `spotify_repeat` | `mode` : string | `status` | — |
| `spotify_volume` | `percent` : u8 | `status` | `volume_changed` |
| `spotify_like` | `uri` : string | — | — |
| `spotify_playlist_add` | `uri` : string, `playlist_id` : string | — | — |
| `spotify_play_all` | `playlist_id` : string, `shuffle` : bool | — | — |

### Last.fm

Scrobbling only — there is no playback control in this group. The flow is the same two-step shape as Spotify's: start the flow to get a URL a browser can visit, then hand the resulting session key back with `lastfm_authenticate`. `lastfm_status` reports `ready` and the last `error`, and is the right thing to poll after starting a flow.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `lastfm_status` | — | `enabled`, `api_key`, `session_token`, `ready`, `loved`, `error` | — |
| `lastfm_oauth_start` | — | `url` | — |
| `lastfm_authenticate` | `token` : string | — | — |
| `lastfm_clear` | — | — | — |
| `lastfm_set_config` | `enabled` : bool, `api_key` : string, `api_secret` : string, `session_key` : string, `min_play_secs` : uint32, `min_play_pct` : float | — | `scrobble_config_changed` |
| `lastfm_love` | — | — | — |
| `lastfm_unlove` | — | — | — |

### Podcasts

Subscriptions are per-daemon and held in the config directory; there is no account. Feeds are read one level down: list feeds, then list episodes for a `feed_id`. `podcast_play` takes an episode index rather than an id, so a client that reorders a feed between listing and playing will play the wrong episode — resolve the index from the same response it displayed.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `podcast_status` | — | `status` | — |
| `podcast_feeds` | — | `feeds` | — |
| `podcast_episodes` | `feed_id` : string | `feed_id`, `feed_title`, `episodes` | — |
| `podcast_add_feed` | `url` : string | `feeds` | — |
| `podcast_remove_feed` | `feed_id` : string | — | — |
| `podcast_refresh` | `feed_id` : string | `refreshed` | — |
| `podcast_play` | `feed_id` : string, `episode_index` : int | — | `playback_started` |

### Charts

A cross-provider view. `charts_list` enumerates the registered providers, and the other two take a provider name from it. Tracks come back normalised to the same `TrackInfo` shape the rest of the protocol uses, so a client does not need a per-provider parser.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `charts_list` | `source_id` : string | `charts` | — |
| `charts_sources` | — | `sources` | — |
| `charts_tracks` | `source_id` : string, `chart_id` : string | `tracks` | — |

### YouTube

Search and download are asynchronous and use the poll pattern throughout: start the work, then call the matching `_poll` command until it reports done. This is the only group where a `_poll` command can return either a payload or a bare ack, so a client must handle both. The `youtube` feature is compiled out of some builds, and every command in this group then fails with "youtube support is disabled in this build" rather than being absent from the enum.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `yt_search` | `query` : string, `filter` : YTFilter | `stations` | `custom` with `name` `yt_search_partial` / `yt_search_done` |
| `yt_search_poll` | — | `query`, `results` | — |
| `yt_search_cancel` | — | — | — |
| `yt_resolve_stream` | `url` : string | `info` | — |
| `yt_fetch_playlist` | `url` : string | — | — |
| `yt_playlist_poll` | — | `query`, `results` | — |
| `yt_download` | `url` : string, `title` : string, `channel` : string | — | — |
| `yt_download_poll` | — | `id`, `url`, `title`, `progress`, `status`, `error`, `file_path`, `downloaded_bytes`, `total_bytes`, `rate_bps`, `eta_secs` | — |
| `yt_cancel_download` | `url` : string | — | — |
| `yt_set_config` | `cookie_source` : string, `cookie_file` : string, `js_runtime` : string, `download_dir` : string, `max_concurrent` : uint32 | — | — |

### Sleep timer and scrobbling

The sleep timer counts down on the daemon and emits a tick every second, so a client rendering a countdown should follow `sleep_timer_tick` rather than decrementing locally — the two drift apart the moment a client is paused or reconnects. Scrobbling configuration is global to the daemon, not per-listener.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `set_sleep_timer` | `minutes` : uint32, `stop_immediately` : bool | — | `sleep_timer_tick`, `sleep_timer_expired` |
| `cancel_sleep_timer` | — | — | — |
| `set_scrobble` | `enabled` : bool, `api_key` : string, `session_token` : string, `min_play_secs` : uint32, `min_play_pct` : float | — | `scrobble_config_changed` |

### System

Introspection and lifecycle. `get_status` and `get_status_lite` differ only in how much they carry: the lite form is for a status bar and will not drag the whole state object across the socket. `check_health` is a deeper probe and can be slow. `quit` persists state and closes the connection, so a client should treat a successful response as the last thing it will ever get.

| Command | Request | Response | Emits |
|---------|---------|----------|-------|
| `get_status` | — | `state` | — |
| `get_status_lite` | — | `state` | — |
| `check_health` | — | `report` | — |
| `quit` | — | — | — |
| `ping` | — | — | — |



# NESTED ACTION ENVELOPES

`queue` and `library` are single `cmd` values whose `action` field selects a
variant of a tagged enum. The variant's own fields sit alongside `action` in the
same object — there is no extra nesting level:

```json
{"id": 5, "cmd": "library", "action": "add_to_playlist", "playlist_id": 1, "track_ids": [4, 5]}
```

An unrecognised `action` is rejected as a malformed request rather than ignored,
so a client should treat it as a hard error and not retry it unchanged.

## queue actions

### queue

| Action | Fields |
|--------|--------|
| `list` | — |
| `clear` | — |
| `remove` | `index` : uint64 |
| `move` | `from` : uint64, `to` : uint64 |
| `add` | `paths` : list, `position` : uint64 (optional) |
| `set` | `paths` : list, `start_idx` : uint64 |

`list` is the only read and the only one with a payload. `add` appends unless
`position` is given, in which case it inserts at that index; `set` replaces the
whole queue and takes the index to start playing from, which is the usual way to
hand a finished playlist to the daemon in one call.

## library actions

### library

| Action | Fields |
|--------|--------|
| `scan` | `path` : string |
| `get_tracks` | `filter` : string (optional), `sort` : string (optional) |
| `get_most_played` | `limit` : uint64 |
| `get_recently_played` | `limit` : uint64 |
| `get_recently_added` | `limit` : uint64 |
| `get_playlists` | — |
| `get_playlist_tracks` | `id` : int |
| `create_playlist` | `name` : string |
| `rename_playlist` | `id` : int, `name` : string |
| `delete_playlist` | `id` : int |
| `add_to_playlist` | `playlist_id` : int, `track_ids` : list |
| `import_playlist` | `path` : string, `format` : PlaylistFormatKind |
| `export_playlist` | `playlist_id` : int, `path` : string, `format` : PlaylistFormatKind |
| `get_recent` | `count` : uint64 |
| `sync_covers` | — |
| `sync_lyrics` | — |
| `sync_metadata` | `path` : string (optional) |
| `sync_status` | — |
| `remove_from_playlist` | `playlist_id` : int, `track_id` : int |
| `playlist_dedup` | `playlist_id` : int |
| `playlist_doctor` | `playlist_id` : int |
| `playlist_sort` | `playlist_id` : int, `field` : string |
| `remove_track` | `id` : int |
| `update_metadata` | `track_id` : int, `patch` : MetadataPatch |

Three of these are asynchronous and return before doing their work: `scan`,
`sync_covers`, `sync_lyrics` and `sync_metadata` acknowledge immediately and
report completion as an event. `sync_status` is the only way to ask about a pass
already in flight.

`sync_metadata` with no `path` processes every track whose metadata looks
unreliable — the title equals the filename stem, or artist or album is missing.
With a `path` it processes just that track.

`update_metadata` takes a `patch` object whose fields are all optional; a field
that is absent is left alone, so a client must not send the field it does not
mean to change.

# EVENTS

Events are unsolicited. A client is expected to read them continuously and
tolerate any it does not recognise, because the set grows without a version
bump. `position_changed` is the only one that arrives at a rate worth naming —
the rest are state transitions.

| Event | Payload |
|-------|---------|
| `playback_started` | `track`, `auto_advanced`, `time_pos`, `duration` |
| `playback_paused` | `time_pos` |
| `playback_stopped` | — |
| `track_ended` | — |
| `position_changed` | `time_pos` |
| `duration_changed` | `duration` |
| `volume_changed` | `volume` |
| `mono_changed` | `enabled` |
| `metadata_changed` | `detail` |
| `queue_changed` | `queue`, `cursor` |
| `queue_index_changed` | `index` |
| `repeat_mode_changed` | `mode` |
| `shuffle_changed` | `enabled` |
| `crossfade_changed` | `enabled`, `duration_secs` |
| `crossfade_countdown` | `track`<br>Emitted once when the next track is about to enter crossfade (5s before it begins). The client animates the countdown until the crossfade starts. |
| `loudness_mode_changed` | `mode` |
| `loudness_scan_progress` | `tracks_remaining`, `tracks_total` |
| `loudness_scan_done` | `scanned`, `failed` |
| `pre_gain_changed` | `pre_gain_db` |
| `gapless_changed` | `enabled` |
| `dynamic_mode_changed` | `enabled`, `min_queue_remaining`, `max_history` |
| `scrobble_config_changed` | `enabled` |
| `sleep_timer_tick` | `remaining_secs` |
| `sleep_timer_expired` | — |
| `low_power_changed` | `enabled` |
| `audio_device_changed` | `name` |
| `eq_preset_changed` | `preset` |
| `eq_enabled_changed` | `enabled` |
| `reverb_changed` | `enabled`, `room_size` |
| `custom` | `name` : string, plus the sub-type's own fields |
| `spotify_status_changed` | Spotify link state changed (e.g. an OAuth link flow completed). |
| `lastfm_status_changed` | — |
| `spectrum_changed` | `levels` |
| `waveform_changed` | `samples`, `stereo`<br>Time-domain waveform ring (interleaved L/R) plus a stereo flag, for the Wave/Stereo visualizer modes. Decimated on the decode/stream threads so a ~5.5 kHz ring reaches the client at ~30 Hz. |
| `radio_title_changed` | `title`<br>A live ICY/Shoutcast stream published a new `StreamTitle` (or cleared it, `None`). |
| `radio_tracks_changed` | `list`, `artist`<br>The playing station's published tracklist changed: first fetch, a refresh, or the entry on air advancing. Carries the whole list so the read-only queue and the now-playing artist split stay in step. |
| `network_status_changed` | `online`<br>Generic internet connectivity changed, as measured by the daemon's bounded TCP probes (1.1.1.1:443, then gstatic.com:443). Independent of any provider link state; drives the footer `Network` module. |
| `provider_error` | `provider`, `message`<br>A provider failed in a way the user has to act on, as opposed to a track that merely is not playable.  Provider-side rejections used to reach the user as silence: librespot logs a rejected audio-item request to its own logger, the resulting `Unavailable` event was consumed to end the source, and the queue moved on — so a token that Spotify refuses produced no sound and no message. This carries the diagnosis instead. |
| `heartbeat` | — |


Under `custom`, the `name` field selects a sub-type whose remaining fields
vary. `custom` names observed in the daemon: `daemon_quitting`, `backend_error`,
`audio_error`, `audio_backend`, `sync_done`, `library_scan`, `library_changed`,
`cover_art`, `lyrics`, `sleep_timer_deferred`, `youtube_search`, `event_channel`,
`gtm`. The set is open — a client must ignore names it does not know rather than
treating one as an error, since new sub-types are added without a protocol bump.
`gtm/src/shared/ipc.rs` is the only complete list.

`heartbeat` is emitted at least every 15 seconds during active playback. It
carries no payload and exists so a client can tell a silent daemon from an idle
one.

# SEE ALSO

**gtmd**(1), **gtm**(1)

# AUTHORS

prjctimg <prjctimg@outlook.com>

# BUGS

Report bugs to <https://github.com/prjctimg/gtm/issues> or by email to
<prjctimg@outlook.com>.

# COPYRIGHT

Copyright (c) 2026 prjctimg.

This is free software released under the GPL-3.0 license. See the LICENSE
file for the full license text.
