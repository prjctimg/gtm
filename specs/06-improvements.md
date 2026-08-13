# Spec 06 — Improvements

Status: **Planned** — six improvements grouped into two categories.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 6.1 — Improve Now Playing display

**File**: `gtm/src/ui.rs`, `gtm/src/footer.rs`

### 6.1.1 — Empty album title
When track album is unavailable, render empty string instead of "Unknown Artist" in Now Playing:
- In `ui.rs` now-playing section (wide variant, lines 506-524 and narrow variant, lines 575-592)
- In `footer.rs` `render_title()` (lines 384-410) — when `t.artist.is_empty()`, return empty string instead of "Unknown Artist"

### 6.1.2 — Remove "Library" title
Remove the "Library" title from the left pane sidebar since it's redundant:
- In `ui.rs` library pane rendering (lines ~1019-1031)
- The sidebar should just show the queue title, not "Library"

### 6.1.3 — Up next track title in Now Playing
Show up next track title in the Now Playing section below the progress indicator:
- Next icon (▶) + track title (from queue_cache, not current track)
- Implementation: modify the now playing render to include up-next track info

### 6.1.4 — Remove over-comprehensive help text
- Trim help text to bare minimum
- **Decision deferred to user**: Which keybindings/text to keep is TBD — ask user which items to retain
- Default proposal (user may override):
  - Keep only: q (quit), ? (help), space (play/pause), n (next), p (prev), ← → (navigate), s (stop)
  - Keep tab navigation + basic list actions (j/k, enter, delete)
  - Remove: extended keyboard help, detailed menu descriptions, etc.
  - The help bar should be minimal: only keybinding shortcuts
- User will specify which specific keybindings and text to keep during implementation

### 6.1.5 — Shell completion descriptions
Ensure all new commands have descriptions for shell completions:
- `gtm -v` → version only, no copyright declaration
- `gtm -vv` → version plus copyright declaration
- `--version` flag → version only

### 6.1.6 — Per-crate READMEs
- Each crate (gtm, gtm-core, gtm-audio, gtmd, gtm-mpris, release-gen) gets its own concise README
- Main README reuses the copyright declaration from the root README
- README content is straight to the point

---

## 6.2 — Better CLI logging

**File**: `gtm/src/cli.rs`, `gtm/src/app.rs`, `gtm-core/client.rs`

### 6.2.1 — Verbose output instead of "ok"
When verbose is true, show contextual feedback:

`gtm next` → output:
```
Now Playing: [Track title]
[Artist name] - [album name]
```

Preserve formatting (tabs and indentation) and color the output.

The key change is in `cli.rs:197-643` — each `CliCommand` match arm now shows contextual info instead of just "ok".

### 6.2.2 — --stream flag for gtm status
Add `--stream` flag to `gtm status` command:
- Streams elapsed track time continuously
- Updates now playing information all from CLI (no TUI)
- Output format: "Stream: [title] - [artist] | [elapsed]s / [duration]s | [volume]%"

### 6.2.3 — All commands
The verbose output should work for all commands:
- `play` → shows file path and start position
- `pause` → shows current track and position
- `next` → shows now playing info
- `prev` → shows now playing info
- `seek` → shows seek target and new position
- `volume` → shows new volume level
- `shuffle` → shows shuffle status
- `repeat` → shows repeat mode
- `mute` → shows mute status
- `crossfade` → shows crossfade status
- `queue` → shows queue length and cursor
- `status` → shows full status with optional stream
- `check_health` → shows health report
- `ping` → shows ping result
- `quit` → exits cleanly

### 6.2.4 — Contextual feedback for property modifications
- When `gtm next` is called after a crossfade, show the new track's metadata
- When `gtm prev` is called after a crossfade, show the previous track's metadata
- When a property is modified (e.g., volume), show the new value with context

---

## 6.3 — Changes summary

| Change | Files | Lines |
|---|---|---|
| Empty album title | `ui.rs`, `footer.rs` | 506-524, 575-592, 384-410 |
| Remove Library title | `ui.rs` | sidebar rendering |
| Up next in Now Playing | `ui.rs` | now playing render |
| Trim help text | `ui.rs`, `keymap.rs` | help section |
| Shell completion descriptions | `cli.rs` | all command variants |
| Per-crate READMEs | `README.md` (main) + new READMEs per crate | root + crates |
| Verbose CLI output | `cli.rs`, `app.rs` | each command output |
| --stream flag | `cli.rs` | status command |
| All new commands have descriptions | `cli.rs` | CliCommand enum |
| Version handling | `main.rs`, `cli.rs` | `gtm -v`, `gtm -vv`, `--version` |