# Reactive Theme — Architecture & Implementation Instructions

Status: **implemented**. Superseded in part by [`REACTIVE_TOKENS.md`](REACTIVE_TOKENS.md),
which extends this: extraction moved to gtmd as described here, and derivation
moves with it, so a client maps roles instead of doing colour maths.

This is the authoritative spec for how the reactive theme works. It moves the
palette out of the Neovim plugin and makes **gtmd the single source of truth**,
delivered over the pulse socket, consumed by `gtm.nvim` which derives the
*entire* theme from that palette. The editor colorscheme is no longer used as a
color source — only semantic state colors are preserved.

---

## 1. Problem

Today two things are wrong:

1. **The palette is computed twice.** gtmd never computes a palette. The TUI
   client (`gtm/src/reactive.rs`) computes one for its own chrome, and
   `gtm.nvim` re-extracts a *third* one locally with `ffmpeg`/ImageMagick.
2. **The plugin blends the palette into the editor colorscheme.** `theme.lua`
   does `blend(base.bg, primary, intensity)` for every surface group, so the
   result is a wash of *the user's current theme* rather than the album's
   palette. The background is therefore never the album color, and switching
   colorschemes changes the result.

Not applied correctly = the theme must be **derived entirely from the gtmd
palette**, updated the moment the track changes.

---

## 2. Target architecture

```
 ┌────────┐  cover bytes (any source)   ┌──────────────┐
 │ gtmd   │ ──────────────────────────► │ extract_pal  │──► primary/secondary/tertiary
 │ daemon │                             └──────────────┘            │
 └────┬───┘                                                         │ DaemonEvent::ReactivePalette
      │  pulse socket (msgpack frames)                              ▼
      └──────────────────────────────────────────────► gtm.nvim  derive_theme(palette)
                                                                  → nvim_set_hl (all groups)
```

Rules:

- **gtmd owns the palette.** It is extracted from the same cover bytes gtmd
  already decodes for `GetCoverArt`, using the extractor the TUI already uses.
- **gtmd pushes it on the pulse socket** (`reactive_palette` event), not on the
  command socket, and not inside a cover response.
- **The plugin derives the whole theme from the three colors** and never reads
  the editor colorscheme for surface/accent colors.
- **Update is immediate**: emitted as soon as the cover for the *current track*
  is at hand (on `PlaybackStarted` when a local cover exists, otherwise on the
  next `GetCoverArt` for the current track, which the plugin issues on the same
  event).

---

## 3. Wire protocol

### 3.1 New event (Rust, `gtm/src/shared/ipc.rs`)

Add to `DaemonEvent` (order of fields matters — see 3.2):

```rust
#[serde(rename = "reactive_palette")]
ReactivePalette {
    /// `#rrggbb`; empty strings mean "no palette" (no art / decode failed).
    primary: String,
    secondary: String,
    tertiary: String,
},
```

### 3.2 Encoding (verified empirically against a live daemon)

The pulse frame is `[4-byte big-endian length][msgpack]`. The payload is a
`Vec<DaemonEvent>`; each event is a **flat array** `[tag, field1, field2, ...]`
in declaration order. rmp-serde's default `Serializer` produces this shape for
the internally-tagged enum.

Captured example (`volume_changed`, volume 91):

```
91 92 ae "volume_changed" 5b
│  │  └ tag (fixstr len 14)
│  └ fixarray(2)  → one event, tag + one field
└ fixarray(1)     → the event vector
```

So a palette frame is:

```
91 93 b0 "reactive_palette" a7 "#1d2540" a7 "#e0574a" a7 "#3fb27f"
```

i.e. `[["reactive_palette", "#1d2540", "#e0574a", "#3fb27f"]]`.

### 3.3 Emission rules (gtmd)

- Compute the palette with `gtm::shared::reactive::extract_palette(&bytes)`.
- Emit **only for the current track**: guard on `inner.state.current_track`
  matching the requested `track_id` / `path`. A stale fetch for a track the
  user has already left must be dropped (no late palette).
- Emit on:
  1. `PlaybackStarted` — if the track has a readable local `cover_path`, read
     the file and emit immediately (no client round-trip).
  2. `Cover::get(...)` — after the bytes are resolved, when the request is for
     the current track. This covers remote/Spotify art and cheaply re-asserts
     the palette.
- If extraction fails, emit `ReactivePalette { primary: "", .. }` (empty
  strings). Clients treat empty as "clear".
- Do **not** emit for album/artist art (`Cover::artist`) or library browsing.

Cost: a 64×64 median-cut over cached bytes, single-digit ms, only on track
change. No gating flag is required; it is idempotent.

### 3.4 Decoding (gtm.nvim, `lua/gtm/events.lua`)

```lua
local WANTED = { ..., reactive_palette = true }

local EVENT_FIELDS = {
    ...,
    reactive_palette = { "primary", "secondary", "tertiary" },
}
```

`normalize_event` already fills `primary`/`secondary`/`tertiary` from the flat
array. No other decoder change is needed — the shape is what it already reads.

---

## 4. Daemon implementation (gtmd + shared)

### 4.1 Share the extractor (DRY)

`extract_palette` + `ReactivePalette` + the color helpers currently live in
`gtm/src/reactive.rs`, which also holds `derive_theme` (ratatui-specific).

- Move `ReactivePalette`, `extract_palette`, `rgb_to_hsv`, `hue_distance`,
  `hue_shift`, `hsv_to_rgb`, `luminance` into a new
  `gtm/src/shared/reactive.rs`; add `pub mod reactive;` to
  `gtm/src/shared/mod.rs`.
- `gtm/src/reactive.rs` keeps `derive_theme` and re-exports the moved items so
  `crate::reactive::{ReactivePalette, extract_palette}` keeps compiling.
- `gtmd` then calls `gtm::shared::reactive::extract_palette` — it must **not**
  add `color-thief` to its own `Cargo.toml` (it comes through the `gtm` dep).

### 4.2 Hook points

- `gtmd/src/daemon/mod.rs`: in the `PlaybackStarted` path, after
  `state.current_track` is set and before/after the event is pushed, spawn the
  palette work (non-blocking — never `.await` image decode on the select loop).
- `gtmd/src/daemon/cover.rs` (`Cover::get`): after the bytes are resolved,
  compute + push.
- Add a private helper, e.g. `Daemon::push_palette(inner, bytes_or_none)` that
  extracts and `push_event`s. Keep it terse (rules limit identifiers to three
  underscore-separated segments).

### 4.3 Rules

- Work on `dev` only (`git pull --rebase` first); tagged commits go to `main`.
- `cargo check` for verification — do not run the full workspace build locally.
- Terse commit messages; no session comments; symbol-level doc comments only.
- No inline `crate::…`/`gtm::…` paths in function bodies — import at the top.

---

## 5. Plugin implementation (gtm.nvim)

### 5.1 Delete local extraction

Remove from `lua/gtm/theme.lua`: `TOOLS`, `T.extract`, `T.quantize`,
`T.select`, and the ffmpeg/ImageMagick dependency. The palette now arrives from
gtmd. `fetch_cover` stays (it feeds the float art and triggers gtmd's palette
emission via `get_cover_art`).

### 5.2 Subscribe

In `lua/gtm/init.lua`, register a handler instead of (or in addition to) the
track hook's extraction:

```lua
events.on("reactive_palette", function(ev)
    if not theme.is_enabled() then return end
    local pal = theme.from_hex(ev.primary, ev.secondary, ev.tertiary)
    if pal then theme.apply(pal) else theme.clear() end
end)
```

Track change (`playback_started`) still calls `get_cover_art` so gtmd recomputes
and re-emits; the plugin does **not** extract anything itself. On
`playback_stopped` / `track_ended`, call `theme.clear()`.

### 5.3 Derive the whole theme from the palette

All math lives in `theme.lua`. Input: `primary`, `secondary`, `tertiary`
(`{r,g,b}` 0–255). No `T._snap`/`T._base` color is used as a *color source*;
the snapshot remains only to restore and to source **semantic** groups.

```
H, S, V = hsv(primary)               -- album hue drives every surface
light   = luminance(primary) > 140    -- album brightness picks the polarity

-- Surface ramp (a tinted neutral built from the album hue, NOT the theme)
dark :  s' = S*0.45 ; v0=0.060  v1=0.090  v2=0.120  v3=0.170
light:  s' = S*0.25 ; v0=0.975  v1=0.945  v2=0.910  v3=0.850

bg0  = hsv(H, s', v0)   -- Normal
bg1  = hsv(H, s', v1)   -- floats / pmenu / statusline
bg2  = hsv(H, s', v2)   -- cursorline / folds / tabline sel
bg3  = hsv(H, s', v3)   -- visual / selection
sel  = mix(bg0, accent_primary, 0.30)   -- cursorline/visual alt
line = mix(bg0, accent_primary, 0.18)   -- borders, winseparator

-- Foreground / dim
dark :  fg = hsv(H, S*0.12, 0.90)  fg_dim = hsv(H, S*0.12, 0.60)
light:  fg = hsv(H, S*0.15, 0.16)  fg_dim = hsv(H, S*0.15, 0.44)

-- Accents: nudge each away from extreme luminance for readability on bg
accent_primary   = adjust(primary,   light)   -- target lum 170 dark / 70 light
accent_secondary = adjust(secondary, light)
accent_tertiary  = adjust(tertiary,  light)
```

`adjust` is the existing helper (blend toward white/black by at most 0.35).

Group mapping — every non-semantic global group is written to an explicit
value (never a blend of the old theme):

| Groups | Value |
| --- | --- |
| `Normal`, `NormalNC` | fg=`fg`, bg=`bg0` |
| `NormalFloat` | fg=`fg`, bg=`bg1` |
| `FloatBorder`, `WinSeparator`, `VertSplit` | fg=`line`, bg=`bg1`/`bg0` |
| `FloatTitle` | fg=`accent_primary`, bg=`bg1`, bold |
| `FloatFooter` | fg=`fg_dim`, bg=`bg1` |
| `CursorLine`, `CursorColumn`, `ColorColumn` | bg=`bg2` |
| `Visual` | bg=`bg3` |
| `LineNr`, `SignColumn` | fg=`fg_dim`, bg=`bg0` |
| `CursorLineNr` | fg=`accent_primary`, bg=`bg2` |
| `Comment`, `NonText`, `Whitespace`, `SpecialKey` | fg=`fg_dim` |
| `Search`, `IncSearch`, `CurSearch` | bg=`sel`, fg=`bg0` |
| `Pmenu` | fg=`fg`, bg=`bg1` |
| `PmenuSel` | fg=`fg`, bg=`bg3` |
| `StatusLine` | fg=`fg`, bg=`bg2` |
| `StatusLineNC` | fg=`fg_dim`, bg=`bg1` |
| `TabLine` | fg=`fg_dim`, bg=`bg1` |
| `TabLineFill` | bg=`bg0` |
| `TabLineSel` | fg=`fg`, bg=`bg2` |
| `Folded` | fg=`fg_dim`, bg=`bg2` |
| `ROLE_PRIMARY` set | fg=`accent_primary` |
| `ROLE_SECONDARY` set | fg=`accent_secondary` |
| `ROLE_TERTIARY` set | fg=`accent_tertiary` |
| `GtmIcon`, `GtmBar` | fg=`accent_primary`, bg=`bg1` |
| `GtmTitle` | fg=`fg`, bg=`bg1` |
| `GtmArtist`, `GtmTime` | fg=`accent_secondary`, bg=`bg1` |
| `GtmAlbum` | fg=`accent_tertiary`, bg=`bg1` |
| `GtmBarEmpty`, `GtmVolume`, `GtmHint` | fg=`fg_dim`, bg=`bg1` |
| **semantic families** | **untouched** (see 5.4) |

Treesitter: skip groups whose snapshot is a `link` (they follow their tinted
target). For standalone `@…` groups, map via the base name (`@function.builtin`
→ `Function` → `accent_primary`).

### 5.4 Semantics

`error`/`warn`/`info`/`hint`, `Diff*`, `Diagnostic*`, `Spell*`, `Todo`,
git-sign groups keep their colorscheme values. These encode meaning and must
not track album art. Everything else is 100% palette-derived.

> This is the one intentional exception to "the entire theme comes from gtmd".
> If a fully album-tinted semantic set is ever wanted, add
> `theme.semantics = "preserve" | "palette"` (default `preserve`).

### 5.5 Persistence & restore

- Snapshot all global groups on `setup` and on every `ColorScheme` (the scheme
  resets highlights; we keep its semantic colors and re-apply the palette).
- Track every group written in `T._touched`; `restore()` writes the snapshot
  back and clears the palette.
- `theme.scope = "global" | "float"` (default `global`) is retained: `float`
  only writes the float's own groups.

---

## 6. Edge cases

| Case | Behaviour |
| --- | --- |
| No artwork / decode fails | gtmd emits empty palette; plugin clears |
| Track changes fast (A→B→C) | daemon guard drops palettes for non-current tracks; plugin keeps previous until B/C arrives |
| Paused | palette retained (no clear on pause) |
| Stopped / track ended | plugin clears on `playback_stopped` / `track_ended` |
| Daemon restart / reconnect | plugin re-requests cover on reconnect; gtmd re-emits |
| Colorscheme changed while playing | snapshot new scheme (semantics) + re-apply stored palette |
| `theme.reactive = false` | handler ignores events; nothing written |

---

## 7. Verification (acceptance)

Daemon (Rust):

- [ ] `cargo check -p gtmd -p gtm` is clean.
- [ ] With a local track playing, connecting to `gtmd.pulse` yields a
      `reactive_palette` frame; hexdump starts `91 93 b0 72656163746976655f70…`.
- [ ] Skipping tracks never delivers a palette for the previous track.
- [ ] The TUI's own reactive theme is unchanged (refactor is behaviour-neutral).

Plugin (headless, no live daemon needed — drive `events` directly):

- [ ] Feed a `reactive_palette` event → `Normal` bg equals `bg0`, an accent
      group equals `accent_primary`.
- [ ] `Normal.bg` is **not** a function of the pre-existing colorscheme:
      applying the same palette under two different base schemes yields the
      same `Normal.bg`.
- [ ] `Error`/`Diff*`/`Diagnostic*` are unchanged by the palette.
- [ ] `ColorScheme` re-applies the palette and keeps semantics from the new
      scheme.
- [ ] `theme.clear()` restores every touched group exactly.

---

## 8. Out of scope

- Changing what the TUI derives (`derive_theme` stays ratatui-only).
- Album/artist art theming.
- Persisting the palette across plugin reloads (re-derived on connect).
