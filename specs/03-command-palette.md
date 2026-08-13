# Spec 03 — Better Command Palette

Status: **Planned** — fix Enter dispatch, add missing commands, ensure all CLI commands available.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 3.1 — Fix Enter dispatch reliability

**File**: `gtm/src/app.rs` (command palette implementation), `gtm/src/keymap.rs:176-183`

### 3.1.1 — Root cause
- The command palette uses `OpenOverlay(PickerId::CommandPalette)` via `OpenOverlay` in keymap.rs
- When the user presses Enter while the palette is open, the `Select` action is dispatched but may not always trigger
- This likely stems from `Select` being bound to `KeyContext::List` but not properly handling the palette open state

### 3.1.2 — Fix
- In `app.rs`, when the Command Palette picker is open, handle `Select` action as a direct palette selection (not list-only)
- Add an explicit `Select` handler in `app.rs` for the command palette mode
- Ensure the palette's `Select` action is dispatched correctly even when the picker is in a "command" state

### 3.1.3 — Test
- Add a test case: press Enter while palette is open, verify command is selected

---

## 3.2 — Add missing commands to palette

**File**: `gtm/src/app.rs` (palette entry building)

### 3.2.1 — Add commands:
- `progress` preset — cycle through progress styles
- `visualizer` preset — cycle through visualizer styles
- `design` preset — cycle through design preset types
- `audio_settings` — quick audio settings picker (to be implemented)

Add these to the command palette's available commands list.

---

## 3.3 — Ensure all CLI commands available

**File**: `gtm/src/app.rs` (command palette mapping)

The command palette must show all `CliCommand` variants (from `gtm/src/cli.rs:55-195`). The mapping from CLI command names to palette entries should be:

| CLI Command | Palette Entry Name |
|---|---|
| `play` | Play |
| `next` | Next |
| `prev` | Prev |
| `stop` | Stop |
| `queue` | Queue |
| `status` | Status |
| `search` | Search |
| `config` | Config |
| `metadata_sync` | Metadata Sync |
| `lyrics` | Lyrics |
| `favorites` | Favorites |
| `yt_search` | YT Search |
| `yt_poll` | YT Poll |
| `playlist` | Playlists |
| ... | ... |

All CLI commands (including `Config`) should have descriptions in the palette.

---

## 3.4 — Shell completion descriptions

**File**: `gtm/src/cli.rs:193-195` (CliCommand enum)

Each `CliCommand` variant needs a `help` description for shell completions. Currently the enum has no help text. The clap derive macro should auto-generate help from the `help` attribute, or we can add explicit help strings to each variant.

Example:
```rust
Play {
    /// Play a track at an optional start position
    #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    path: String,
    #[arg(value_name = "SECONDS")]
    start_pos: Option<f64>,
},
```

Add `help` fields to each variant:
```rust
Play {
    /// Play a track at an optional start position (default: 0.0)
    #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
    path: String,
    #[arg(value_name = "SECONDS")]
    start_pos: Option<f64>,
}
```

---

## 3.5 — Summary of commands to ensure in palette

- `play`, `play_pause`, `pause`, `stop`, `next`, `prev`, `seek`, `volume`, `shuffle`, `repeat`, `mute`, `crossfade`, `queue`, `queue_add`, `queue_remove`, `queue_move`, `queue_clear`, `queue_set`, `scan`, `tracks`, `playlists`, `create_playlist`, `delete_playlist`, `add_to_playlist`, `import_m3u`, `export_m3u`, `recent`, `metadata_sync`, `favourites`, `favourite_add`, `favourite_remove`, `yt_search`, `yt_poll`, `yt_cancel`, `yt_resolve`, `lyrics`, `search`, `status`, `check_health`, `ping`, `quit`, `config`
- All must have `help` descriptions for shell completions