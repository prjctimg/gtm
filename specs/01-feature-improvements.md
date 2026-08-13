# Spec 01 — Feature Improvements

Status: **Planned**. All items are application-code changes. Green gates after
each item: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace`.

---

## B1 — Library rows: titles only (everywhere)

Hide artist in every track row. Decision: applies everywhere incl. drill-downs.

- Flat track table — `gtm/src/ui.rs:1019-1031`:
  change `format!("{}  {}", track.artist, track.title)` to `track.title.clone()`.
- Detail/browse drill-down — `gtm/src/ui.rs:838-842`: same, titles only.
- Spotify playlist detail — `gtm/src/ui.rs:777-781`: drop the
  `"{artist} \u{2014} {name}"` em-dash concat; use `tr.name` only.
- Keep the `is_current` `> ` prefix and the accent/BOLD current-row styling
  unchanged.
- Do NOT change the Now Playing pane's `Artist:` row or the footer status bar.

## B2 — Lyrics time-sync: top-anchored scroll

The active-line driver already exists (`gtm/src/app.rs:891-913` sets
`lyrics_scroll` when `!lyrics_manual_scroll`), and the highlight exists at
`ui.rs:1791-1798`. The only change is the window math at `ui.rs:1779-1785`:

- Replace the centered window
  (`current - visible / 2` clamp) with a **top-anchored** window:
  `scroll_start = current.min(total.saturating_sub(visible))` when
  auto-scrolling, so the active verse is the first visible row and the next
  lines scroll in below it (one line at a time as it advances).
- Keep centering for the manual-scroll mode (see B7) or make manual scrolling
  free-range; decide during implementation and note it.
- With `.wrap()` (B7) the window must be computed over **display rows**, not
  logical LRC lines. See the display-row mapping in B7.

## B3 — One source of truth for track meta (daemon)

Problem: `finish_crossfade` (`gtmd/src/daemon.rs:1274-1309`) stamps
`state.current_track` / `PlaybackStarted` with the bare queue `TrackInfo`
(filename stem, empty artist/album), because `queue::resolve_track`
(`gtmd/src/queue.rs:22-45`) constructs it without a library lookup. `cmd_play`
(daemon.rs:1422-1570) is the only path that re-resolves from SQLite.

Implementation:

1. Add `resolve_track_meta(inner: &DaemonInner, path: &Path, dur: f64) ->
   TrackInfo` in `gtmd/src/daemon.rs`:
   - canonicalize the path;
   - exact `Library::track_by_path` (`gtmd/src/library.rs:476-489`);
   - substring DB match fallback (today inline at daemon.rs:1487-1497);
   - final fallback: cleaned filename stem + `Unknown Artist`/`Unknown Album`;
   - stamp `duration = dur`.
2. Refactor `cmd_play` (daemon.rs:1456-1554) to call it (replace the inline
   re-resolution).
3. `finish_crossfade` (daemon.rs:1277-1289): call it for the `next` track
   instead of using the queued `TrackInfo` verbatim.
4. `queue::resolve_track` (`gtmd/src/queue.rs:22-45`): canonicalize the path.
   Optionally attempt a library lookup at queue time; if it stays cheap (no
   new DB open per track), do it — otherwise rely on step 3.

Acceptance: after a crossfade auto-advance onto a library track, the daemon's
`PlaybackStarted` payload contains the real title/artist/album (verify with
`gtm status --json`).

## B4 — Now Playing: drop artist from line 1

- `gtm/src/ui.rs:506-524` (wide) and `575-592` (narrow): Row 0 becomes
  `format!("{}{}", fav_prefix, display_title)` — remove the
  `\u{2014}` em-dash + `display_artist` concat.
- Keep Row 1 `Artist: {artist}` (ui.rs:536-541).
- The `display_artist` variable can then be removed (or reused) in both spots.

## B5 — tachyonfx `evolve_into` animation

New dependency: `tachyonfx = "0.22"` in `gtm/Cargo.toml` (ratatui-0.30
compatible — verified against lock at `ratatui 0.30.2`).

Design:

- Add a transient `track_anim_trigger: bool` to `App`
  (`gtm/src/app.rs`, near `last_track_path_display: Option<String>` at line
  231).
- Set it to `true` on the first frame (initial render, `app.rs:604`) and in the
  track-change block (`app.rs:673-684`) whenever `last_track_path_display`
  changes (covers auto-advance via `PlaybackStarted` AND manual). Clear it
  after the animated frame renders.
- Add a render helper (in `gtm/src/ui.rs`):
  `render_evolving(f, area, widget, active, theme)`:
  - if `!active` → `f.render_widget(widget, area)` (normal refresh path);
  - if `active` → render `widget` into a scratch `ratatui::buffer::Buffer` for
    `area`, iterate cells with `tachyonfx::CellIterator`, run
    `tachyonfx::fx::evolve_into(EffectTimer::from_millis(350,
    TimingFunction::QuadInOut), theme.bg.into(), CellFilter::All)` via an
    `FxState::animate` step, then blit the buffer to the frame.
- Apply it to:
  - library list item labels (the flat table row `Line`s built at
    ui.rs:1019-1044),
  - the Now Playing title/artist spans (ui.rs:506-534 / 575-601).
- Refresh frames (the 16 ms draw loop, `app.rs:943`) must render without the
  effect — only the initial and track-change frames animate.

API note: confirm `CellIterator`/`FxState::animate`/`fx::evolve_into` signatures
against tachyonfx 0.22 docs (`cargo doc -p tachyonfx`). `ratatui::style::Color`
must convert to `tachyonfx::fx::Colour` (there is a `From` impl in 0.22).

## B6 — Crossfade eagerness

Two-part (decision committed in `specs/README.md`):

1. Trigger margin — `gtmd/src/daemon.rs:1367`:
   `(dur - pos) <= cf.duration_secs as f64 + 0.5` → `+ 0.15`.
2. EOF-aware swap — `gtm-audio/src/mixer.rs:666-714` (`step_crossfade`): at
   `progress >= 1.0`, do not unconditionally `old_active.stop()`. Only stop it
   when its `active_remaining()` is `<= ~0.05` (or the source reports
   finished); otherwise leave it playing to natural EOF while the new track is
   already at full volume. Verify the paused/standby bookkeeping that follows
   still holds (is_a_active flip, start_time/start_pos reset at mixer.rs:690-711).

Acceptance: crossfade onto a song's last ~seconds no longer cuts the tail;
manual listen test with a 3 s crossfade and a track whose final note runs to
EOF.

## B7 — Lyrics pane: focusable + scrollable + wraps

- Add `lyrics_pane_focus: bool` to `App` (`app.rs:180` region, beside
  `library_pane_focus`).
- `FocusLeft`/`FocusRight` (`app.rs:2041-2048`): when `show_lyrics` and on the
  Library tab, cycle focus left-pane → right-pane → lyrics pane.
  `Back` (`app.rs:2050-2067`) exits lyrics focus to the track pane.
- `MoveUp`/`MoveDown`/`PageUp`/`PageDown`/`Top`/`Bottom` (`app.rs:2075-2143`):
  when `lyrics_pane_focus`, move `lyrics_scroll` (clamped to the last visible
  logical line) and set `lyrics_manual_scroll = true` — this currently-dead
  flag (`app.rs:892`) then keeps the time-sync driver from fighting the user.
- `ui.rs:1801`: add `.wrap(Wrap { trim: false })` to the lyrics `Paragraph`.
- Because wrapping changes logical→display row mapping, add a helper that maps
  `(logical_line_index, pane_width)` → `(display_start_row, display_height)`
  for the wrapped text, then:
  - highlight the active line's full wrapped block (`fg_bright`+BOLD),
  - scroll window computed over display rows (used by B2's auto-scroll too),
  - cap manual scroll at the row that shows the last logical line.
- Consider a `[l]`/`Enter`-from-lyrics binding to enter lyrics focus; arrows
  are already bound in `KeyContext::Normal` (`gtm/src/keymap.rs:195-224`).

## Acceptance

- `gtm` TUI: All Tracks list shows titles only; lyrics highlight follows
  playback and the pane scrolls up one line per verse; lyrics pane accepts
  focus and arrows scroll it with wrapping; Now Playing line 1 is title-only;
  library list animates on startup and on each track change only; crossfade no
  longer cuts the tail; `gtm status --json` shows clean meta after a
  crossfade auto-advance.
