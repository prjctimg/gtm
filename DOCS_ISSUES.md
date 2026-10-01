# DOCS_ISSUES

Findings from an audit of [`gtm.docs`](https://github.com/prjctimg/gtm.docs) against
this repository at `9b0c5a7` (0.2.83, branch `dev`).

Every item below was checked against source. Line references are to the docs repo
unless stated otherwise. This file exists so the prose in the published docs can
describe what gtm *does* rather than narrating the state of the codebase while it
was being written — the observations here are for a maintainer to act on, not for a
reader to trip over.

## Sections

- [A. Prose stripped from the docs](#a-prose-stripped-from-the-docs)
- [B. Doc claims that contradict the code](#b-doc-claims-that-contradict-the-code)
- [C. Undocumented features — candidate pages](#c-undocumented-features--candidate-pages)
- [D. Bugs found in gtm.rs](#d-bugs-found-in-gtmrs)

---

## A. Prose stripped from the docs

These described internal or unfinished implementation state. Removed from the docs;
the underlying observation is recorded here.

| Was | Where |
|---|---|
| `gapless` "is currently a no-op placeholder — the flag is stored and restored but no playback, mixer or decoder path consults it, so it has no audible effect today", plus the note that it is reachable only via `DaemonReq::SetGapless` with no CLI command or TUI binding | `crossfade.mdx` |
| `queue-set`'s `--start-idx` "is currently accepted and ignored: playback always starts at index 0" | `playback.mdx` |
| `EqPreset::Custom([f32; 15])` "exists in the state model but is **not exposed in the TUI** — there is no per-band editing UI yet" | `audio.mdx` |
| `playlist-dedup` "in practice can never remove anything — the primary key already prevents duplicates, so it is effectively a position repack" | `library.mdx` |
| The `.m3u8` stub "is **not kept in sync** … never contains a track and is not a usable export" | `library.mdx` |
| The help buffer "is a hand-maintained list … neither complete nor exact" | `tui.mdx` |
| "`check_health` has no default key. The help screen advertises `Alt+H`, but no such binding exists" | `keybindings.mdx` |
| `Alt+p` "shadowed and unreachable by default" for Podcasts | `keybindings.mdx` |
| "note `Alt+p` is already mapped to the Progress Style picker" | `podcasts.mdx` |
| Keybinding contexts "are metadata, not live modes … never dispatched by the keymap at all" | `keybindings.mdx` |
| "There's a lot more involved in setting things up and I decided to just take the naive route because its enough, at least for now." | `benchmark.mdx` |

See also [D](#d-bugs-found-in-gtmrs) — several of these have a root cause in the code.

---

## B. Doc claims that contradict the code

These need a decision: fix the docs, or ship the feature. Listed roughly by severity.

### B1. Playback speed is documented on six pages and does not exist

There is no `speed` or `rate` symbol anywhere in `gtm/src` or `gtmd/src`. No
`CliCommand::Speed`. `AudioSettings` (`gtm/src/shared/state.rs:129`) has no speed
field. `FooterModule` (`gtm/src/footer.rs:80`) has no `Speed` variant.

Documented anyway in: `playback.mdx` (a whole *Playback speed* section, `>` / `<`
keys, `gtm --cli speed`, the 0.25×–2.0× clamp), `keybindings.mdx` (`>` / `<` rows),
`daemon.mdx` (the `state.json` persistence list), `tui.mdx` (the footer module
table), `mpris.mdx` ("playback speed is not mapped"), `configuration.mdx` (footer
preset module lists).

The docs section was removed. **The feature needs restoring or the scope needs
deciding.**

### B2. No visualizer toggle key

`Ctrl+v` is documented as "toggle the visualizer" in `keybindings.mdx` and
`audio.mdx`. There is no `Char('v') + CONTROL` binding in
`gtm/src/keymap.rs`. The visualizer now renders only in Zen mode and during
daydreaming (`gtm/src/ui/chrome.rs:238`).

### B3. Progress style is `Alt+b`, not `Alt+p`

`Alt+p` opens Podcasts; progress style moved to `Alt+b`
(`gtm/src/keymap.rs`, guarded by `podcasts_and_progress_style_are_both_reachable`).
The old shadowing was a real bug and is now fixed — the docs were never updated.
Wrong in `keybindings.mdx`, `theming.mdx`, `tui.mdx`.

### B4. There is no `Alt+t`

`radio.mdx` documents `Alt+t` for the Radio Browse overlay. Only `Alt+r` exists,
and it already carries the tag/country sections.

### B5. The library layout has no visualizer pane

`tui.mdx` opens with an ASCII diagram showing a visualizer pane beside Now Playing.
`Render::library` (`gtm/src/ui/chrome.rs:790`) lays out left pane + results +
optional lyrics only. The visualizer is fullscreen Zen or daydream. **The diagram
was replaced** with one matching the real layout.

### B6. Last.fm setup has no callback server

`metadata-sources.mdx` describes a loopback callback on `127.0.0.1:8991`, a
five-minute window, and `$GTM_LASTFM_PORT` to override. None of those exist. The
CLI prints an authorize URL and polls status for 180 s
(`gtm/src/cli.rs:1704`). The authorize URL carries a temporary token
(`gtmd/src/providers/lastfm/mod.rs:612`, with a test asserting it).
`$GTM_LASTFM_PORT` was also absent from the env-var table in `configuration.mdx`,
which was the tell. **The section was rewritten** to match.

### B7. Lyric nudge is 0.1 s per press, not ±120 s

`nudge_lyrics_offset(±0.1)` at `gtm/src/app/keys.rs:776`. `lyrics.mdx` claimed
"nudge the lyric clock by up to ±120 s".

### B8. Counts are stale

| Claim | Actual | Source |
|---|---|---|
| 14 library categories | 15 — `Browse` was added | `LIBRARY_CATEGORIES`, `gtm/src/app/mod.rs:68` |
| 14 `left_pane_lists` names | 15, and `Browse` is missing from the allowlist | `configuration.mdx`, `clean_left_pane` |
| 22 footer modules | 21, and no `Speed` | `gtm/src/footer.rs:80` |
| 51 command-palette actions | 54 | `gtm/src/ui/command.rs` |

### B9. `cover-art.mdx` links to a section that does not exist

`[Limitations](#_limitations)` — `cover-art.mdx` has no Limitations section. Its
frontmatter description also promises "limitations".

### B10. Benchmark README link is wrong

`benchmark.mdx` and `src/pages/Benchmark.tsx` link to `BENCHMARK.md` at the repo
root. The file is at `.bench/BENCHMARK.md`.

### B11. Crossfade default duration

`crossfade.mdx` said crossfade defaults to 6 s (true, `DaemonState::new`) and that
the CLI "defaults to 7 when omitted" (also true, `gtm/src/cli.rs:614`) without
reconciling the two. Reworded to name both cases.

---

## C. Undocumented features — candidate pages

Real, working features with no page. Ordered by how much surface each would absorb.

1. **Charts** — Apple Music, Deezer and Spotify Top Charts
   (`gtmd/src/providers/charts/{apple,deezer}.rs`; Deezer landed in `9b0c5a7`).
   Today: one table row in `library.mdx` and two list mentions.
2. **Browse** — the 15th sidebar category, backed by Deezer: free-text search,
   artist pages, album tracklists (`gtmd/src/providers/browse.rs`). Completely
   undocumented, and it is the reason Spotify artist/album browsing is absent
   (Spotify's public API no longer exposes artist contents).
3. **Notifications** — 10 categories (`NotifType::ALL`, `gtm/src/app/notify.rs:29`),
   each with a `floating` / `footer` / `off` mode, plus a history overlay and
   floating cards. The three `[extensions]` switches belong here.
4. **Daydreaming** — the visualizer takes over after `daydream_secs` of idle
   (default 60, `gtm/src/app/prefs.rs:139`), set in Settings → System.
5. **Discord Rich Presence** — `discord_id` in config, numeric-only validation
   (`gtmd/src/config.rs:290`), cleared on daemon quit, reachable via `Alt+x` →
   Discord (`PickerId::DiscordSetup`).
6. **Downloads** — yt-dlp subprocess, 2-concurrent cap, EMA-smoothed footer
   progress, `~/Music/gtm/downloads`, post-download library scan. Currently
   squeezed into `youtube.mdx`.
7. **IPC reference** — this repo ships `docs/man/gtmd-ipc.1.md`, which the site
   surfaces nowhere. The intro pitches "integration with other terminal tools (via
   IPC)"; this is the page that makes that pitch real.
8. **Troubleshooting** — the sharpest entry is that **one invalid enum value in
   `config.toml` discards the entire file and silently falls back to defaults**.
   `configuration.mdx` warns about it in a `caution` callout; it deserves a page.

Also worth a mention: a printable one-page keymap card, and shell completions /
man pages, which are currently one bullet in `install.mdx`.

### Two undocumented keys

`daydream_secs` and `discord_id` / `discord_app_id` are readable, persisted config
keys with no documentation at all. `configuration.mdx` gained rows for the first.

---

## D. Bugs found in gtm.rs

Independent of the docs. Found while auditing to verify claims above.

1. **`gtm/src/ui/help.rs` advertises `Alt+P` twice** — once for Progress Style
   (should be `Alt+b`) and once for Podcasts. The `?` buffer therefore tells the
   user two different things about the same key. `gtm/src/ui/command.rs` repeats
   the same stale `Alt+H` claim.
2. **`Alt+H` is advertised for `check_health` but no such binding exists.** The
   action, the CLI command, and the advertised key all exist; only the default
   binding is missing. Pressing the advertised key does nothing.
3. **`i` (track info) and `l` (fetch lyrics) work but appear in no doc page.**
4. **`queue-set --start-idx` is parsed, threaded through IPC, and discarded.**
   `gtmd/src/queue.rs:325` takes `_start_idx` and ignores it. Either honour it or
   drop the flag — silently accepting an argument and ignoring it is worse than
   either.
5. **`.m3u8` playlist stubs are written but never updated.** `gtmd/src/library.rs:409`
   writes a two-line stub at create time and nothing rewrites it, so the file in
   the data dir is never a usable export.
6. **`playlist-dedup` cannot remove anything.** The `(playlist_id, track_id)`
   primary key already prevents duplicates; the command is a position repack with a
   name that promises otherwise.
7. **`gapless` is persisted and never read.** `set_gapless` writes state and emits
   `GaplessChanged`; no playback, mixer or decoder path consults the flag. It has no
   CLI command or TUI binding either.
8. **No WMA decoder, but `wma` is in the accepted extension list.**
   `gtmd/src/queue.rs:348` accepts it for `queue-add`, so a `.wma` file enters the
   queue cleanly and fails only at decode time.
9. **`EqPreset::Custom([f32; 15])` is unreachable from the UI** and never written by
   anything, so per-band EQ is state-only.