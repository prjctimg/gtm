# Reactive Tokens — moving colour derivation into the daemon

Status: **implemented**, daemon and shared halves. Follows on from
`REACTIVE_THEME.md`, which moved *extraction* to gtmd. This moves *derivation* as
well. The `gtm.nvim` half below is still outstanding — that plugin is not in this
repository — so do not cut a daemon release for this until it has shipped, which
is the sequencing §8 of this document asks for.

Two deviations from the text below, both because the code did not match the
proposal:

- **`derive` is `derive(pal, dark)` as specified, and the TUI's ambient wash
  stays out of it.** `intensity` is a client preference the daemon has never
  heard of, so folding it into the shared derivation would have made every
  editor client's theme depend on a TUI setting. The TUI applies its own wash on
  top, in `derive_theme`, which is the one layer that has the setting.
- **The checklist's `gtm.spec` item does not apply.** `dist/rpm/gtm.spec` is RPM
  packaging and carries no protocol text; `docs/man/gtmd-ipc.1.md` is the
  canonical document and carries the new event.

`REACTIVE_THEME.md` stopped the palette being computed three times, but left every
client computing its own **ramp** from the same three colours. `gtm.nvim` still
does HSV maths, contrast adjustment and surface derivation in Lua; the next
editor plugin will do it again, differently. This document makes gtmd the single
source of the derived colours too, so a client only maps names.

---

## 1. Problem

`reactive_palette` carries three hex strings. Each consumer derives the rest:

| Consumer | Derives |
|---|---|
| `gtm` TUI | `derive_theme` — accent adjust, wash, `fg_dim` contrast |
| `gtm.nvim` | `theme.lua` — `hsv_to_rgb`, `rgb_to_hsv`, `mix`, `adjust`, `derive_ctx` |
| future plugins | unknown, and each will differ |

Duplicated maths drifts. Two editors on the same track render different
backgrounds, which undercuts the point of a shared reactive theme.

---

## 2. Target

gtmd resolves a palette into a complete **role → colour** set and pushes it. A
client receives the final colours and does no arithmetic at all.

```
 cover bytes ──► extract_palette ──► 3 colours ──► derive_tokens ──┐
                                        (exists)      (NEW)         │
                                                                    ▼
                                              DaemonEvent::ReactiveTheme
                                                     { ver, dark, light }
                                                                    │
              ┌─────────────────────────────────────────────────────┤
              ▼                            ▼                        ▼
        gtm TUI                     gtm.nvim                 other editors
     map roles to                 map roles to              map roles to
     ratatui Colours              highlight groups          their own groups
```

**No colour maths below the wire.** Clients hold a name→role table, which is
data.

---

## 3. What the daemon can and cannot own

### 3.1 It cannot own group names

`@function.builtin.lua` (Neovim), `function.builtin` (Helix) and
`editorToken.foreground` (VS Code) are different namespaces. No payload fixes
that. The daemon owns **roles**; each client owns **names**.

The client-side residue is a table:

```lua
local ROLE = { Normal = "bg0", Comment = "fg_dim", String = "accent2", ... }
```

That is the whole remaining cost. No maths.

### 3.2 Polarity cannot be a daemon guess

The daemon does not know whether the connected editor is light or dark. It
cannot ask: the pulse socket is a **single broadcast stream shared by every
connected client**, so there is no per-client channel to answer on. Two editors
with different polarities connected at once cannot both be served by one flag.

Therefore **both polarities are sent in one event** and the client picks. Cost is
~1 KB per track change, which is nothing against the 60 Hz spectrum frames
already on the same socket. This avoids a `set_theme_ctx` round trip, and avoids
the wrong-polarity flash that a round trip would cause on the first track.

### 3.3 Semantic colours are not artwork-driven

Deriving `error` from album art makes diagnostics harder to scan — the exact
problem `REACTIVE_THEME.md` decided to avoid in the plugin. The token set
therefore *includes* semantic roles so every editor is consistent about them, but
the daemon derives them from **fixed, contrast-checked hues**, not from the
palette. A client may ignore them and keep its colorscheme's own values; the
plugin does exactly that today and will continue to.

### 3.4 Intensity

The TUI has a user-tunable 5-step background wash. Baking one intensity in would
lock editors to it. Instead the ramp is sent washed *and* unwashed:
`bg0..bg3` carry the album wash, `bg_flat` is the same ramp at zero saturation.
A client that wants accents without a tinted background reads `bg_flat` — still
no arithmetic.

---

## 4. Shared module

New `gtm/src/shared/theme.rs`, sibling to `shared/reactive.rs`, **no ratatui**:

```rust
/// A resolved ramp: every colour a client needs, by role.
pub type Roles = BTreeMap<&'static str, String>;

/// Build the full role set for one polarity. Pure; no globals, no I/O.
pub fn derive(pal: &ReactivePalette, dark: bool) -> Roles;

/// The role set as it crosses the wire, both polarities, versioned.
pub struct ReactiveTheme {
    pub ver: u8,
    pub dark: BTreeMap<String, String>,
    pub light: BTreeMap<String, String>,
}
```

The ramp maths moves here from `gtm/src/reactive.rs`. `derive_theme` stays in the
client but is rewritten to consume `derive()` rather than re-deriving, which is
the DRY win inside this repo — the accent adjust and `fg_dim` contrast logic is
already duplicated between `reactive.rs` and the plugin today.

`shared/reactive.rs` keeps `extract_palette`, `to_hex`, `from_hex` and the colour
primitives; `shared/theme.rs` consumes them.

---

## 5. Wire protocol

New event in `gtm/src/shared/ipc.rs`:

```rust
#[serde(rename = "reactive_theme")]
ReactiveTheme {
    /// Token-set version. Bumped when a role is renamed or removed.
    ver: u8,
    /// Roles for a dark surface.
    dark: BTreeMap<String, String>,
    /// Roles for a light surface.
    light: BTreeMap<String, String>,
},
```

The pulse frame is a StructTuple array, so this is:

```
[["reactive_theme", 1, {"bg0":"#0d0d12", ...}, {"bg0":"#f6f4f1", ...}]]
```

`BTreeMap` serialises as a MessagePack map, which decoders handle as a table —
`gtm.nvim`'s `vim.mpack` path already does, and the tag peek is unaffected
because it only reads the front of the frame.

### 5.1 Roles

43 roles. Grouped by what they are for, not by which client wants them.

| Group | Roles |
|---|---|
| Surface | `bg0` `bg1` `bg2` `bg3` `bg_flat` |
| Foreground | `fg` `fg_dim` `fg_bright` |
| Accent | `accent` `accent2` `accent3` |
| Line | `line` `line_focus` |
| Selection | `sel_bg` `sel_fg` |
| Cursor | `cursor` `cursor_line` `cursor_line_nr` |
| Search | `search_bg` `search_fg` |
| Syntax | `comment` `string` `keyword` `function` `type` `variable` `constant` `number` `boolean` `operator` `punctuation` `identifier` |
| Semantic | `error` `warning` `info` `hint` `success` |
| Diff | `diff_add` `diff_delete` `diff_change` `diff_text` |
| Misc | `heading` `link_url` |

### 5.2 Compatibility

Additive. `reactive_palette` is unchanged and still broadcast, so the TUI and any
third-party client keep working. A client that does not know `reactive_theme`
never sees it.

A new client against an old daemon sees `reactive_palette` only, and must fall
back — see §7.

---

## 6. Emission

In `gtmd`, next to the existing extraction, both hook points reuse the palette
already computed there rather than extracting twice:

- `PlaybackStarted` with a readable local `cover_path`
- `Cover::get` when the request is for the track on air

Rules, unchanged from `REACTIVE_THEME.md`:

- Emit only for the track on air; guard on `on_air(state, id, path)`. A late
  fetch for a track the user has left must not repaint.
- Extraction failure emits `reactive_palette` with empty strings as today, and
  emits no `reactive_theme` — nothing to derive from.
- Never on artist/album art or library browsing.
- Non-blocking: the decode stays in `spawn_blocking`.

One extraction, two events, back to back.

---

## 7. Client obligations

1. **Feature-detect, never version-gate.** If `reactive_theme` has not arrived,
   fall back to local derivation. Do not refuse to theme because the daemon is
   old — that regresses every current user.
2. **Fall back to `reactive_palette`.** Three colours are strictly less
   information than the role set, so the fallback derives from those.
3. **Prefer daemon roles for every non-semantic group.** Semantic groups keep the
   editor's colorscheme values (§3.3).
4. **Keep name→role as data.** If a client needs a role the set lacks, ask for it
   to be added rather than computing it locally.

`gtm.nvim` implements exactly this and is the reference client. Its local
derivation is retained as the fallback path for one release, then deleted.

---

## 8. Work

### gtmd + shared

- [ ] `shared/theme.rs`: `Roles`, `derive(pal, dark) -> Roles`, `ReactiveTheme`
- [ ] Port the ramp from `gtm/src/reactive.rs`; delete the duplicates
- [ ] Rewrite `derive_theme` to consume `derive()` (client-side, no behaviour change)
- [ ] `ReactiveTheme` variant in `DaemonEvent`
- [ ] Emit beside both existing palette hook points
- [ ] Unit tests: contrast floors per polarity, accent distinctness, both
      polarities produce a complete role set
- [ ] `docs/man/gtmd-ipc.1.md`: document the event and the role list
- [ ] `gtm.spec` protocol: same, if the spec repo is the canonical reference

### gtm.nvim (this repo)

- [ ] Subscribe to `reactive_theme`
- [ ] `theme.lua`: rename the derived context to roles; one `apply` path, two
      token sources
- [ ] `:checkhealth gtm` reports which source is live
- [ ] Docs updated for the new event and the retained fallback
- [ ] Delete the local derivation once the daemon ships and a release has passed

### Sequencing

The event is additive, so the daemon can ship first with no plugin change. Land
the plugin subscription before the daemon so no daemon release is wasted, and
delete the fallback only after a tagged daemon is widely deployed.

---

## 9. Limits, stated plainly

- **Group names cannot be centralised.** A client keeps a name→role table. This
  is the irreducible cost and it is data, not logic.
- **The pulse socket is a broadcast.** Per-client polarity is not possible
  without a protocol change; sending both polarities is the cheap fix and has a
  real 2× payload cost.
- **Semantic roles are fixed-hue, not artwork-driven.** Consistency is bought;
  artwork-driven diagnostics are deliberately not offered.
- **The role set is a floor, not a ceiling.** A client needing a role the set
  lacks must wait for a daemon release rather than deriving locally. Acceptable
  while the set is small; revisit if clients start forking it.
- **Extending `reactive_palette` instead of adding an event was rejected.** It
  would break positional decoding for existing clients. A new tag keeps every
  current consumer working untouched.
- **A map-valued field is new on this wire.** StructTuple events have carried
  only scalars so far. Any client with a hand-rolled msgpack decoder must handle
  maps before it can read this event — check yours.
- **The plugin's fallback is kept on purpose.** Deleting it before the daemon is
  deployed would break theming for anyone on an older daemon.