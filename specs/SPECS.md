# SPECS.md — Implementation Specifications

> Generated from the PROMPT file. Each section is self-contained so an agent can
> pick up any task independently.

---

## 1. Fix Spotify Authentication Flow

**Problem:** The Spotify login/sync flow does not properly authenticate the user and
sync playlist data as expected. Myx handles this directly via librespot credentials
+ Web API; gtm.rs uses an IPC-split daemon architecture with PKCE OAuth.

**Key files:**
- `gtmd/src/oauth.rs` — PKCE OAuth flow (port 8990, S256)
- `gtmd/src/spotify.rs` — `SpotifyManager`: token persistence, sync, search, playback
- `gtm-core/src/spotify.rs` — shared types (`SpotifyStatus`, `SpotifyPlaylist`, `SpotifyTrack`)
- `gtm-core/src/secret.rs` — keyring/file token storage
- `gtm/src/app.rs:540-549` — TUI OAuth state fields
- `gtm/src/ui.rs:2424` — SpotifyLink picker rendering
- `gtm/src/cli.rs:189` — CLI `SpotifyAction` subcommand
- `gtmd/src/daemon.rs:1056+` — IPC handler for `Spotify*` requests

**How Myx does it (reference):**
- `engine::credentials()` loads/creates librespot `Credentials` from a cached token file
- `WebApi::init()` does OAuth PKCE before terminal init (single-instance lock prevents race)
- Auth happens BEFORE the alternate screen opens, so browser prompts are visible
- Token refresh uses a `TokenCallback` that re-persists on every refresh

**Implementation plan:**
1. Verify the `oauth_start` flow: browser opens → user authorizes → callback on port → `set_token` called → `sync` runs. Check that the 5-min deadline in `listen_for_auth_code` is sufficient for large libraries.
2. After `set_token`, ensure `init_client` properly rebuilds the rspotify client with the new token and the `TokenCallback` persists refreshed tokens.
3. Verify `sync` (60s timeout) paginates through all playlists. The `oauth_start` handler deliberately has no artificial timeout on post-link sync — confirm this works.
4. Check the `SpotifyLink` picker flow: the TUI shows "Waiting for you to finish login in your browser…" — verify the OAuth URL is correctly displayed and the redirect port is configurable.
5. Ensure `SpotifyClear` properly wipes token from both keychain and file, and that re-login works.

**Verification:** `cargo build --workspace` succeeds. Manual test: run `gtm`, press `Alt+S`, complete OAuth flow, verify playlists sync.

---

## 2. Fix Playback Speed Slowdown on Track Switch

**Problem:** When switching tracks (manual next/prev or auto-advance), playback
experiences a perceptible slowdown. No time-stretching code exists in the codebase
(confirmed by grep for `time.?stretch|tempo|speed|rate_change|rubberband|rubato`).

**Key files:**
- `gtm-audio/src/mixer.rs:363-424` — `load_active`, `load_active_decoded`
- `gtm-audio/src/buffer.rs` — `PREBUFFER_SAMPLES = 132300` (1.5s), `BUFFER_CAPACITY_SAMPLES = 529200` (6s)
- `gtm-audio/src/mixer.rs:704-766` — `poll()` with underrun grace (100ms)
- `gtmd/src/daemon.rs:125-236` — `Cmd::play` (local file path)
- `gtmd/src/daemon.rs:241-260` — `Cmd::play_stream` (Spotify path)
- `gtmd/src/stream.rs` — librespot `PcmStreamSource` → rodio chain, `CHANNEL_CAPACITY = 64`

**Root cause analysis:**
- `load_active` stops the old player, starts a decode thread, waits for prebuffer (1.5s), then appends to the rodio `Player`. The 100ms `UNDERFLOW_GRACE` in `poll()` delays `Finished` events.
- For streamed (Spotify) tracks, `play_stream` calls `stream.load(uri, ...)` which may reinitialize the librespot `Session`/`Player`, causing a gap.
- The `start_time` is reset to `None` then set on `play()`, but there may be a window where the position tracker lags.
- The `pending_pause` fade-out (150ms) adds latency when switching mid-play.

**Implementation plan:**
1. In `load_active`, avoid the full `stop()` + re-init cycle when transitioning to the next track. Instead, use the standby player pattern (A/B swap) that already exists for crossfade but apply it to next-track transitions.
2. Reduce `PREBUFFER_SAMPLES` for subsequent tracks — the initial 1.5s prebuffer is only needed on first play; subsequent tracks can start with less buffered data (e.g., 0.5s = 44100 samples).
3. In `play_stream`, reuse the existing librespot `Session`/`Player` instead of calling `stream.load()` which may recreate them. The `StreamManager` already tracks `session_token` for reconnection.
4. Remove or reduce the `UNDERFLOW_GRACE` from 100ms to 30ms — the grace period is meant to avoid false `Finished` on brief buffer underruns, but 100ms is excessive.
5. Ensure `start_time` is set atomically with `play()` to avoid position tracking lag.

**Verification:** `cargo build --workspace`. Manual test: play a track, switch to next mid-playback, verify no perceptible speed change or gap.

---

## 3. Fix Footer Timezone (Two Hours Behind)

**Problem:** The footer `Time` module displays UTC time instead of local time, making
it two hours behind (for UTC+2 timezone users).

**Key files:**
- `gtm/src/footer.rs:661-795` — custom `strftime` function and `render_time`

**Root cause:** The `strftime` function at line 671 uses `SystemTime::now().duration_since(UNIX_EPOCH)` which gives UTC epoch seconds. It then computes `hour = secs_of_day / 3600` from the raw UTC seconds without applying any timezone offset.

**Implementation plan:**
Replace the manual UTC computation with `chrono::Local::now()` which is already available
as a dependency via `gtm-core` (chrono is in `gtm-core/Cargo.toml`).

In `gtm/src/footer.rs`:
1. Add `use chrono::Local;` at the top.
2. In `render_time` (line 662), replace the call to `strftime(&app.footer_time_format)` with a direct `Local::now().format(&app.footer_time_format).to_string()`.
3. The custom `strftime` function (lines 671-795) and `day_of_year` (lines 797-805) can be removed since `chrono` handles all formatting specifiers natively (`%H`, `%M`, `%S`, `%Y`, `%m`, `%d`, `%a`, `%A`, `%b`, `%B`, `%j`, `%p`, `%P`, `%I`, `%e`, `%y`).

**Note:** If chrono's `Local` is not accessible from the `gtm` crate (it's in `gtm-core`), add `chrono` as a direct dependency of the `gtm` crate in `gtm/Cargo.toml`.

**Verification:** `cargo build --workspace`. Verify the footer time matches `date` command output.

---

## 4. Fix Reactive Theme Background Gutter Artifacts

**Problem:** The reactive theme produces weird gutter artifacts — areas of the TUI
background that don't match the reactive blend, creating visible seams.

**Key files:**
- `gtm/src/reactive.rs:96-143` — `derive_theme` blends bg, pane_bg, elevated_bg, picker_bg
- `gtm/src/theme.rs` — `AppTheme` struct with all color fields
- `gtm/src/ui.rs` — main renderer, all pane/section renderers

**Root cause:** Some rendering paths use hardcoded or stale base theme colors instead
of the reactive theme. The "more vibrant" approach (higher blend factors in
`derive_theme`) should be kept; the issue is inconsistent application.

**Implementation plan:**
1. Audit every place in `ui.rs` that creates a `Style::default().bg(...)` or reads `app.theme.bg` directly. Ensure all background rendering uses the current reactive theme (which is already applied to `app.theme` via `derive_theme`).
2. Specifically check: `fill_pane`, `pane_header`, footer `right_bg`, notification overlay background, and the library pane border areas.
3. In `footer.rs:315-322`, the `right_bg` calculation uses `app.theme.bg` which should already be reactive — verify.
4. Ensure the library pane, track info card, and up-next card all use `app.theme.pane_bg` or `app.theme.bg` consistently.
5. The key fix is ensuring the gutter (unfilled area between left and right footer groups, and the borders/margins of panes) uses the reactive background, not the base theme background.

**Verification:** `cargo build --workspace`. Test with reactive_theme = true and various cover art to verify no gutter artifacts.

---

## 5. Clear Reactive Theme From Previous Track When New Track Has No Cover

**Problem:** When playing a new track that has no cover art, the previous track's
reactive color palette persists in the UI.

**Key files:**
- `gtm/src/reactive.rs` — `derive_theme`, `extract_palette`
- `gtm/src/app.rs` — track change handling, `reactive_palette` field (line 584)

**Implementation plan:**
1. In the track-change handler (wherever `PlaybackStarted` events are processed), check if the new track has cover art.
2. If no cover art is available, reset `app.reactive_palette` to `None` and reapply the base theme to `app.theme`.
3. The relevant code is where `reactive_palette` is set after fetching cover art — ensure the "no cover" path explicitly clears it.

**Verification:** `cargo build --workspace`. Play a track with art, then switch to a track without art — verify theme reverts to base.

---

## 6. Fix Now Playing Progress Layout

**Problem:** The track progress indicator and elapsed time are rendered inline (same
line). They should be on separate lines: progress bar on one line, elapsed time below.

**Key files:**
- `gtm/src/ui.rs:1750+` — `track_info_in_pane` function
- `gtm/src/ui.rs` — now-playing card rendering

**Implementation plan:**
1. In the now-playing track info section, find where the progress bar and time are rendered together in a single `Line`.
2. Split into two `Line`s: first line contains the progress bar, second line contains the elapsed time (and optionally total duration).
3. Ensure the spacing accounts for the new line height.

**Verification:** `cargo build --workspace`. Visual check of now-playing section.

---

## 7. Default Footer Preset: Show All Playback States, No Track Title

**Problem:** The Default footer preset should show ALL playback states (Playing,
Paused, Stopped) and NOT show the current track title (leave Title for custom
footers).

**Key files:**
- `gtm/src/footer.rs:113-134` — Default preset definition
- `gtm/src/footer.rs:453-477` — `render_playback` (already handles all 3 states)

**Current Default preset (line 114-134):**
```rust
left: vec![
    FooterModule::Playback,   // keeps
    FooterModule::Title,      // REMOVE
    FooterModule::Repeat,
    FooterModule::Shuffle,
    FooterModule::Volume,
    FooterModule::EqPreset,
    FooterModule::KeyAction,
    FooterModule::Notification,
    FooterModule::SleepTimer,
],
```

**Implementation plan:**
1. Remove `FooterModule::Title` from the Default preset's `left` vec.
2. `render_playback` already renders Playing (▶), Paused (⏸), and Stopped (■) states — no changes needed there.
3. The "Full" preset can optionally keep `Title` if desired.

**Verification:** `cargo build --workspace`. Check that the Default footer no longer shows the track title.

---

## 8. Fix Multiselect: Shift+Up/Down Arrows, Highlight, Footer Count

**Problem:** Multiselect should be triggered by Shift+Up/Down arrows, selected items
should be highlighted, and the count should show in the footer.

**Key files:**
- `gtm/src/keymap.rs:551-558` — current multiselect binding (`v` key)
- `gtm/src/app.rs:4690-4708` — `ToggleMultiselect` handler
- `gtm/src/app.rs:613,616` — `multiselect_mode`, `selected_indices` fields
- `gtm/src/ui.rs:3875` — command palette entry
- `gtm/src/footer.rs` — footer rendering

**Implementation plan:**
1. Add Shift+Up and Shift+Down keybindings that, when in multiselect mode, toggle the item under the cursor in `selected_indices` and move the cursor.
2. In the library list rendering (in `ui.rs`), check if each row's index is in `selected_indices` and apply a distinct highlight style (e.g., a checkbox prefix or different selection color).
3. Add a footer section (or modify the existing `Queue` module area) to display the multiselect count, e.g., `[3 selected]`, when `multiselect_mode` is true and `selected_indices` is non-empty. This should appear near the library stats area in the footer.

**Verification:** `cargo build --workspace`. Test multiselect with Shift+Up/Down, verify highlighting and footer count.

---

## 9. Fix Petty Toggle Notifications: Use Footer, Not Floating

**Problem:** Simple toggle notifications (multiselect count, state toggles) use the
floating window notification overlay. They should use inline/footer notifications
instead. The floating notification is reserved for longer/richer messages.

**Key files:**
- `gtm/src/app.rs:372-407` — `NotifMode` enum (Floating, Footer, Off)
- `gtm/src/app.rs:416+` — `Notification` struct
- `gtm/src/app.rs:553` — `footer_notification` field
- `gtm/src/ui.rs:177` — `notification_overlay` floating toast
- `gtm/src/footer.rs:445-451` — `render_footer_notification`

**Implementation plan:**
1. Find all places where simple toggle notifications are sent (e.g., multiselect ON/OFF, repeat mode changes, shuffle toggle, mute toggle).
2. Instead of calling `notify_typed()` (which creates floating toasts), set `app.footer_notification = Some((msg, expires))` directly for these simple messages.
3. Keep floating notifications for: error messages, library operations (add to queue/playlist), Spotify login status, download progress — anything with more context.
4. The distinction: if the notification is a single short phrase (≤30 chars), use footer. If it has richer content, use floating.

**Verification:** `cargo build --workspace`. Toggle multiselect, repeat, shuffle — verify they appear in the footer, not as floating toasts.

---

## 10. Fix Muted Text Visibility Against Reactive Background

**Problem:** Elements like elapsed time and other muted helper text are not visible
enough against the current reactive background.

**Key files:**
- `gtm/src/theme.rs` — `AppTheme.fg_dim` field
- `gtm/src/reactive.rs:96-143` — `derive_theme` (does not adjust `fg_dim`)
- `gtm/src/footer.rs` — `render_time`, `render_progress` use `app.theme.fg_dim`

**Implementation plan:**
1. In `derive_theme` (`reactive.rs`), also blend `fg_dim` based on the reactive palette. The current `fg_dim` may be too close to the reactive background color.
2. Ensure `fg_dim` has sufficient contrast against the reactive `bg`. Calculate the luminance difference and adjust if needed (minimum 4.5:1 contrast ratio for readability).
3. In `footer.rs`, ensure all "muted" text uses `app.theme.fg_dim` (which will now be reactive-aware).
4. Check `ui.rs` for any hardcoded `Color::Rgb` values used for dim/muted text.

**Verification:** `cargo build --workspace`. Test with various cover art and both dark/light themes.

---

## 11. Command Palette: Material Design Icons with Emoji Toggle

**Problem:** The command palette uses emojis. Users should be able to toggle between
emojis and Material Design Icons (MDI) via config TOML. Default should use MDI.

**Key files:**
- `gtm/src/ui.rs:3855-3897` — `COMMAND_PALETTE_COMMANDS` (currently emojis)
- `gtm/src/app.rs:77-105` — `Prefs` struct
- `gtm/src/app.rs:119-120` — `default_time_format`

**Implementation plan:**
1. Add a `icon_style: String` field to `Prefs` (default: `"mdi"`). Options: `"mdi"` (Material Design Icons via nerd fonts), `"emoji"`.
2. Create two versions of `COMMAND_PALETTE_COMMANDS`: one with MDI icons (nerd font codepoints like `\u{f04ba}` for play) and one with emojis (current).
3. At render time, check `app.current_prefs.icon_style` and select the appropriate command list.
4. Define a mapping of each command to its MDI icon codepoint. Common MDI icons:
   - Play/Pause: `\u{f04ba}` / `\u{f04cd}`
   - Next: `\u{f04ad}`, Prev: `\u{f04a8}`
   - Volume: `\u{f057e}`, Mute: `\u{f0580}`
   - Repeat: `\u{f0577}`, Shuffle: `\u{f0578}`
   - Search: `\u{f057a}`, Queue: `\u{f056e}`
   - Settings: `\u{f0493}`, Equalizer: `\u{f0570}`
   - etc.
5. Update the config docs in `wiki/Configuration.md`.

**Verification:** `cargo build --workspace`. Set `icon_style = "mdi"` and `"emoji"` in config, verify command palette uses the correct icons.

---

## 12. Fix Equalizer Picker: Description Below Visualization

**Problem:** The equalizer picker shows preset descriptions inline per item. They
should be below the visualization in the preview, updating as the user scrolls.

**Key files:**
- `gtm/src/ui.rs:4615-4763` — `render_equalizer_picker`
- `gtm/src/ui.rs:4765-4800+` — `eq_preset_preview`

**Current layout:**
```
 > Flat    — Neutral, uncoloured response
   Normal  — Balanced all-rounder
   Pop     — Vocal-forward with a lively top end
   ...
──────── Flat ────────
▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁  ← EQ visualization
```

**Desired layout:**
```
 > Flat
   Normal
   Pop
   Rock
   ...
──────── Flat ────────
▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁  ← EQ visualization
Neutral, uncoloured response  ← description below viz
```

**Implementation plan:**
1. In `render_equalizer_picker`, remove the ` — {desc}` suffix from each list item (line 4704-4707). Show only the preset name.
2. In the preview area (below the visualization), add a new line for the selected preset's description. The description text should update as the user scrolls the list.
3. The preview area height may need to be increased from 4 to 5 rows to accommodate the description line.

**Verification:** `cargo build--workspace`. Open equalizer picker, scroll through presets, verify description appears below visualization.

---

## 13. Fix Lyrics: Update on Track Change, Auto-Fetch

**Problem:** Lyrics are not updated when the track changes. They should auto-fetch
if they don't exist and there's an internet connection.

**Key files:**
- `gtm/src/app.rs:900` — `next_lyrics_gen` field
- `gtm/src/app.rs:7111` — `lyrics_are_synced`
- `gtm/src/ui.rs:1520-1600` — `lyrics_pane`
- `gtmd/src/lyrics.rs` — `LyricsManager` with lrclib.net fetch
- `gtm/src/keymap.rs` — `FetchLyrics` (`l` key)

**Implementation plan:**
1. In the track-change handler (where `PlaybackStarted` events are processed), clear the current lyrics (`app.current_lyrics = None`) and trigger a lyrics fetch if:
   - The lyrics pane is visible, OR
   - Auto-fetch is enabled (add `auto_fetch_lyrics: bool` to `Prefs`, default true)
2. The fetch should use the same `LyricsManager` API that the manual `l` key uses.
3. If the fetch fails (no internet, no lyrics found), show "No lyrics found" without error toasts.
4. Ensure the lyrics generation counter (`next_lyrics_gen`) is incremented on track change to invalidate any in-flight fetches from the previous track.

**Verification:** `cargo build --workspace`. Play tracks sequentially — verify lyrics update automatically.

---

## 14. Fix Queue Picker: Update Up-Next Cover on Manual Track Change

**Problem:** When the user manually changes the track from the queue picker, the
cover of the up-next track should update. If the next track has no cover, show
the default.

**Key files:**
- `gtm/src/ui.rs:2739-2860` — `render_queue_picker`, `render_queue_upnext_preview`
- `gtm/src/ui.rs:39-100` — `upnext_card`
- `gtm/src/app.rs` — queue state, cover fetching

**Implementation plan:**
1. When the user selects a track from the queue picker and confirms playback, trigger a cover art fetch for the new "up next" track (the one after the newly playing track).
2. The up-next card rendering should check if cover art is available for the next track; if not, use the default placeholder.
3. Ensure the cover fetch happens asynchronously and the UI updates when the fetch completes.

**Verification:** `cargo build --workspace`. Open queue, select a different track, verify up-next cover updates.

---

## 15. Replace 'linux' Text with Platform Mascot/Logo

**Problem:** The footer shows "linux" as the system name. Instead, show the platform
mascot (Tux penguin for Linux) next to the audio backend, combining them into one
module.

**Key files:**
- `gtm/src/footer.rs:655-659` — `render_system` (returns `std::env::consts::OS`)
- `gtm/src/footer.rs:641-653` — `render_backend` (audio backend name)

**Implementation plan:**
1. Modify `render_system` to return a platform icon instead of "linux":
   - Linux: Tux penguin `\u{1f427}` (penguin emoji) or nerd font `\u{f17c}` (Linux)
   - macOS: `\u{1f34e}` (apple) or nerd font `\u{f302}` (Apple)
   - Windows: `\u{1f5a5}` or nerd font `\u{f87a}`
2. Merge the System and Backend modules into a single display: e.g., `🐧 rodio` or `🐧 pulseaudio`.
3. This can be done by combining the two modules into one rendered badge, or by modifying `render_system` to include the backend name.

**Verification:** `cargo build --workspace`. Check footer shows platform icon + backend.

---

## 16. Fix Library Motions in Multiselect Mode

**Problem:** Library motions (add to queue, add to playlist, delete) should work on
multiple selected items in multiselect mode, with a confirmation prompt since the
operation involves many items.

**Key files:**
- `gtm/src/app.rs:4710-4750` — `AddToQueue`, `AddToPlaylist` handlers (already check `multiselect_mode`)
- `gtm/src/app.rs:6448-6477` — command palette versions

**Current behavior:** These already check `multiselect_mode` and iterate over `selected_indices`. The missing piece is the confirmation prompt.

**Implementation plan:**
1. Before executing the bulk operation, show a confirmation prompt: "Add N tracks to queue? [y/N]".
2. The prompt should be displayed as a floating notification or a dedicated prompt UI element.
3. Wait for `y`/`n` keypress before proceeding.
4. Apply the same pattern to delete operations in multiselect mode.

**Note:** Task 19 (keeping prompts displayed until keypress) is related and should be implemented alongside this.

**Verification:** `cargo build --workspace`. Enter multiselect, select multiple tracks, trigger add to queue — verify confirmation prompt appears.

---

## 17. Fix Input Prompts: Keep Displayed Until Keypress

**Problem:** Prompts that require user input (confirmation dialogs) disappear too
quickly. They should remain displayed until the expected key has been pressed.

**Key files:**
- `gtm/src/app.rs` — notification/prompt handling
- `gtm/src/ui.rs` — notification overlay rendering

**Implementation plan:**
1. For prompts requiring user input (delete confirmation, multiselect bulk operations, etc.), use a stateful prompt mode instead of timed notifications.
2. Add a `pending_prompt` field to `App` that holds the prompt message and expected key responses.
3. Render the prompt as a persistent overlay (not a timed notification).
4. Clear the prompt only when one of the expected keys is pressed.
5. The prompt should not auto-expire like `Notification.expires_at`.

**Verification:** `cargo build --workspace`. Trigger various prompts, verify they persist until keypress.

---

## 18. Fix Commit Hash in CI Build

**Problem:** The commit hash should be added during the build step in CI.

**Key files:**
- `gtm/build.rs:20-35` — `git rev-parse --short HEAD` → `VERGEN_GIT_SHA`
- `.github/workflows/ci.yml` — CI workflow
- `.github/workflows/release.yml` — release workflow

**Current state:** `gtm/build.rs` already runs `git rev-parse --short HEAD` and emits `VERGEN_GIT_SHA`. This works locally because `.git/HEAD` exists.

**Potential issue:** In CI, the checkout may use a shallow clone or detached HEAD, which could produce an incorrect or missing hash.

**Implementation plan:**
1. In `.github/workflows/ci.yml`, ensure `actions/checkout@v7` fetches enough history for `git rev-parse` to work (add `fetch-depth: 0` or at least `fetch-depth: 1` which should be sufficient for short SHA).
2. In `.github/workflows/release.yml`, verify the checkout step also has sufficient depth.
3. In `gtm/build.rs`, add a fallback: if `git rev-parse` fails (e.g., `.git` not present in a source tarball), emit a hash from an environment variable (`VERGEN_GIT_SHA` env var set by CI) or `"unknown"`.
4. The build.rs already falls back to `"unknown"` on failure, so this is mainly about ensuring CI sets it correctly.

**Verification:** `cargo build --workspace`. Check CI builds show correct commit hash in About panel.

---

## 19. Change Build to Use GCC for Release, Clang for Termux

**Problem:** Release builds should use gcc; clang should only be used for Termux
builds and when running `cargo build` in Termux.

**Key files:**
- `gtm/build.rs:80-108` — linker selection (mold detection)
- `.github/workflows/release.yml:173-176` — RUSTFLAGS with `-C linker=clang`
- `.cargo/config.toml` — linker config
- `scripts/build/musl-in-container.sh:10` — RUSTFLAGS with clang
- `scripts/build/arch-in-container.sh:10` — RUSTFLAGS with clang

**Current state:** All Linux release builds use `clang + mold` via RUSTFLAGS. The build.rs probes for mold locally.

**Implementation plan:**
1. In `.github/workflows/release.yml`, change the matrix for Linux (non-Android) targets:
   - Replace `link_override: -C linker=clang` with `link_override: -C linker=gcc` (or omit to use default gcc).
   - Keep `mold_arg: -C link-arg=-fuse-ld=mold` for mold support with gcc (gcc also supports `-fuse-ld=mold`).
2. In `scripts/build/musl-in-container.sh` and `scripts/build/arch-in-container.sh`, change `RUSTFLAGS` to use `gcc` instead of `clang`.
3. Keep clang only for Termux/Android builds (which are handled by cargo-ndk and don't use these scripts).
4. In `gtm/build.rs`, the mold detection logic stays the same — it's orthogonal to the compiler choice.

**Verification:** `cargo build --workspace`. Verify CI release builds use gcc. Check the About panel's linker info.

---

## 20. Fix Multiselect: Shift+Up/Down with Visual Highlight and Footer Count

*(This is an expanded version of Task 8 with more implementation detail.)*

**Key files:**
- `gtm/src/keymap.rs` — keybinding definitions
- `gtm/src/app.rs:613-616` — `multiselect_mode`, `selected_indices`
- `gtm/src/ui.rs` — library list rendering
- `gtm/src/footer.rs` — footer modules

**Implementation plan:**
1. **Keybindings:** Add `Shift+Up` → `MultiselectUp` and `Shift+Down` → `MultiselectDown` actions. These move the cursor AND toggle the item's selection state.
2. **Visual highlight:** In the library list rendering, for each row, check if `selected_indices.contains(&row_index)`. If so, render with a checkbox prefix (☑/☐) or a distinct background color.
3. **Footer count:** Add a `Multiselect` footer module (or integrate into existing `Queue` module area) that shows `[N selected]` when `multiselect_mode && !selected_indices.is_empty()`.
4. **Command palette:** Update the entry at line 3875 to mention Shift+Up/Down.

**Verification:** `cargo build --workspace`. Full multiselect workflow test.

---

## 21. Fix Petty Toggle Notifications (Expanded)

*(Expanded version of Task 9.)*

**Specific notifications to route to footer:**
- Multiselect ON/OFF (`app.rs:4696-4707`)
- Repeat mode changes
- Shuffle toggle
- Mute toggle
- Volume changes
- EQ preset changes
- Sleep timer set/cancelled

**Notifications to keep as floating:**
- "Added N track(s) to queue"
- "Added to playlist"
- "Deleted N track(s)"
- Spotify login/sync status
- Error messages
- Download progress

**Implementation plan:**
For each toggle notification, replace:
```rust
self.notify_typed("System", msg, NotificationKind::Info, false, NotifType::Prefs);
```
with:
```rust
self.footer_notification = Some((msg.to_string(), std::time::Instant::now() + std::time::Duration::from_secs(2)));
```

**Verification:** `cargo build --workspace`. Test all toggles — verify footer display, no floating toast.

---

## 22. Fix Muted Text Visibility (Expanded)

*(Expanded version of Task 10.)*

**Key approach:**
1. In `reactive.rs:derive_theme`, add reactive-aware `fg_dim`:
   ```rust
   // Ensure fg_dim contrasts with the reactive bg
   let bg_lum = luminance(&pal.primary);
   let target_dim_lum = if bg_lum > 128.0 { bg_lum - 60.0 } else { bg_lum + 60.0 };
   t.fg_dim = adjust_for_contrast(base.fg_dim, target_dim_lum);
   ```
2. Audit all uses of `fg_dim` in `ui.rs` and `footer.rs` to ensure consistency.
3. For the elapsed time specifically, use `fg_dim` with sufficient opacity/contrast.

**Verification:** `cargo build --workspace`. Visual check across themes and reactive backgrounds.

---

## Dependency Graph

Tasks can be implemented in this order (based on dependencies):

1. **Task 3** (timezone) — standalone, quick win
2. **Task 7** (footer default preset) — standalone
3. **Task 15** (platform mascot) — standalone
4. **Task 12** (EQ picker layout) — standalone
5. **Task 5** (clear reactive theme) — standalone
6. **Task 10/22** (muted text visibility) — standalone
7. **Task 4** (reactive gutter artifacts) — depends on Task 10/22
8. **Task 9/21** (petty notifications) — standalone
9. **Task 6** (now playing layout) — standalone
10. **Task 2** (playback slowdown) — standalone, complex
11. **Task 1** (Spotify auth) — standalone, complex
12. **Task 8/20** (multiselect) — standalone
13. **Task 16** (library motions multiselect) — depends on Task 8/20
14. **Task 17** (persistent prompts) — depends on Task 16
15. **Task 13** (lyrics auto-fetch) — standalone
16. **Task 14** (queue up-next cover) — standalone
17. **Task 11** (command palette icons) — standalone
18. **Task 18** (CI commit hash) — standalone
19. **Task 19** (gcc for release builds) — standalone
