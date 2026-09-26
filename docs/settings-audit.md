# Configuration audit

What `config.toml` actually holds today, what the TUI can change, and which
daemon settings have no surface at all.

`config.toml` is written by the TUI (`gtm/src/app/prefs.rs`, `save_prefs`)
and read by the daemon for exactly two keys (`gtmd/src/config.rs`). Everything
else the TUI changes is a runtime command, not a stored preference — so
"changed in Settings" and "persisted" are not the same thing.

## Where the file is written

`prefs_path()` → `$XDG_CONFIG_HOME/gtm/config.toml`, falling back to
`~/.config/gtm/config.toml`. `ensure_prefs_file()` writes a default file on
first run so `gtm config` always opens something valid.

## Persisted keys (22)

Serialized from the `Prefs` struct. Every one has a `#[serde(default)]`, so a
hand-edited file that omits keys still loads.

| Key | Type | TUI surface |
|---|---|---|
| `theme_name` | string | Settings → System → Theme |
| `theme_mode` | string | Settings → System → Theme Mode |
| `transparent_bg` | bool | Settings → System → Transparent BG |
| `transparent_pickers` | bool | Settings → System → Transparent Pickers |
| `reactive_theme` | bool | Settings → System → Reactive Theme |
| `reactive_theme_intensity` | float | Settings → System → Reactive Intensity |
| `footer_preset_name` | string | Settings → System → Footer Preset |
| `progress_style` | enum | **none** (picker only) |
| `visualizer_preset` | enum | Settings → System → Visualizer |
| `extensions` | table | **none** (file only) |
| `time_format` | string | **none** (picker only) |
| `track_sort` | enum | **none** (picker only) |
| `keybindings` | table | **none** (file only) |
| `notification_modes` | table | Settings → System → Notification Settings |
| `footer_key_action` | `action` \| `keys` | **none** (file only, by design) |
| `cover_provider` | string | Settings → Playback → Cover Source |
| `cover_cache_mb` | int | Settings → System → Cover Cache Size |
| `auto_fetch_lyrics` | bool | **none** (file only) |
| `icon_style` | string | **none** (picker only) |
| `hide_footer` | bool | **none** (file only) |
| `left_pane_lists` | list | **none** (file only) |
| `show_preview` | bool | **none** (file only) |

### Gaps worth closing

- **`extensions`, `auto_fetch_lyrics`, `hide_footer`, `left_pane_lists`,
  `show_preview`** are file-only. `hide_footer` and `show_preview` in
  particular are the kind of thing a user expects to find in Settings.
- **`footer_key_action`** is file-only by intent: it picks whether the footer
  echoes a command's name or the key pressed, which is a muscle-memory choice
  rather than a presentation one. It is carried through `current_prefs`
  verbatim so saving any other setting does not reset it.
- **`progress_style`, `time_format`, `track_sort`, `icon_style`** are changed
  through dedicated pickers rather than the Settings pane, which is a
  reasonable choice but means Settings is not a complete picture of the
  configuration.
- **`keybindings` and `notification_modes` are tables with no schema
  validation.** An unknown notification category is silently ignored; an
  unknown keybinding is silently unused. A typo produces no diagnostic.

## Settings that are NOT persisted

The Settings pane exposes 36 rows across four categories
(`gtm/src/app/theme.rs::category_options`). Most of them send an IPC command
and change daemon runtime state that is not written back to `config.toml`:

| Category | Rows | Persisted? |
|---|---|---|
| YouTube | 4 (cookie source/file, JS runtime, auto download) | no — cookie paths live in the daemon's own `yt-dlp` config |
| Playback | 6 (repeat, shuffle, crossfade, EQ enabled, reverb, cover source) | only `cover_provider` |
| System | 15 | 10 of 15 persist; 5 are commands ("Sync Covers", "Clear Lyrics Cache", …) |
| Spotify | 11 (status, account, playlists, link, sync, unlink, device, next, previous, shuffle, repeat) | no — all daemon/runtime state |

**Consequence:** repeat mode, shuffle, crossfade, EQ, reverb, speed, mono and
gapless all reset to their defaults on daemon restart, even though the TUI
presents them as ordinary settings. That is defensible for transport state
(repeat/shuffle are arguably per-session) but surprising for EQ and crossfade,
which most players persist.

The daemon reads only `cover_provider` and `cover_cache_mb` from this file
(`gtmd/src/config.rs:168-184`). Everything else in the daemon is command-driven.

## Recommendation

1. Add the five file-only toggles that users are most likely to look for
   (`hide_footer`, `show_preview`, `auto_fetch_lyrics`, `extensions`,
   `left_pane_lists`) to the System category.
2. Validate the two tables on load and warn on unknown keys, so a typo is
   visible instead of silent.
3. Decide whether EQ / crossfade / gapless should persist. If yes, they belong
   in `Prefs`; if no, the Settings pane should not present them as ordinary
   preferences.
