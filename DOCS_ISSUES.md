# DOCS_ISSUES

Findings from an audit of [`gtm.docs`](https://github.com/prjctimg/gtm.docs) against
this repository at `9b0c5a7` (0.2.83, branch `dev`).

Every item below was checked against source. Line references are to the docs repo
unless stated otherwise. This file exists so the prose in the published docs can
describe what gtm *does* rather than narrating the state of the codebase while it
was being written — the observations here are for a maintainer to act on, not for a
reader to trip over.

**Re-audited at `d433e51`.** Everything in section D has been fixed, and section A
has been overtaken by it. Several section B key names changed again when the
presentation picker replaced five overlays. Read the **[changed]** and **[fixed]**
markers below rather than trusting the original numbers — several are now wrong in
the *other* direction, and copying them into the docs would reintroduce the very
errors this audit was written to catch.

## Sections

- [Docs-site changes already applied](#docs-site-changes-already-applied)
- [A. Prose stripped from the docs](#a-prose-stripped-from-the-docs)
- [B. Doc claims that contradict the code](#b-doc-claims-that-contradict-the-code)
- [C. Undocumented features — candidate pages](#c-undocumented-features--candidate-pages)
- [D. Bugs found in gtm.rs — all fixed](#d-bugs-found-in-gtmrs--all-fixed)

---

## Docs-site changes already applied

So the next pass does not re-litigate settled decisions:

- `content/interface.mdx` → `content/tui.mdx`, route `/docs/tui`. **The old
  `/docs/interface` URL is not redirected and will 404** — by choice, not
  oversight. Anything still linking to it needs updating (`playback.mdx`,
  `configuration.mdx`, `RULES.md`, and the sitemap, which regenerates on build).
- Sidebar: the "More" group heading is gone. `/docs/` pages and the top-level
  `/install` and `/benchmark` pages render as one continuous list.
- The breadcrumb component was deleted outright rather than reduced to a single
  crumb, since `HeadingSelect` already covers jump-to-top on narrow viewports
  and the `<h1>` sits directly below it on desktop.
- `public/samples/` → `public/media/{static,gif}/`.
- Every item in section A was removed. Every item in section B that could be
  fixed from the docs side was fixed: the counts, key names, the Last.fm flow,
  the lyric nudge, the layout diagram, `Alt+b`, `Browse`, the broken anchors and
  the wrong benchmark README link.
- `daydream_secs` and `discord_id` / `discord_app_id` gained rows in the config
  field reference, and a Discord Rich Presence section was added.

Items in section B that need a decision rather than an edit: **B1** (speed),
**B2** (visualizer toggle). **B3** and **B4** are code fixes — the docs now
describe the code as it should be.

---

## A. Prose stripped from the docs

These described internal or unfinished implementation state. Removed from the docs;
the underlying observation is recorded here.

**Several are no longer true of the code** — section D fixed them, so the honest
fix on the docs side is to describe the behaviour, not to keep hedging. The last
column says which; rows marked *Still true* are accurate as written and the only
thing to do there is leave them recorded.

| Was | Where | Now |
|---|---|---|
| `gapless` "is currently a no-op placeholder — the flag is stored and restored but no playback, mixer or decoder path consults it, so it has no audible effect today", plus the note that it is reachable only via `DaemonReq::SetGapless` with no CLI command or TUI binding | `crossfade.mdx` | **[fixed]** Removed outright: the state field, the FSM method, the IPC request, the event and both man-page rows. It is no longer a feature that does nothing; it is gone. Say nothing about it. |
| `queue-set`'s `--start-idx` "is currently accepted and ignored: playback always starts at index 0" | `playback.mdx` | **[fixed]** The flag, the client parameter, the IPC field and the dead daemon function are removed. `queue-set` now just replaces the queue; **play** is what starts playback. |
| `EqPreset::Custom([f32; 15])` "exists in the state model but is **not exposed in the TUI** — there is no per-band editing UI yet" | `audio.mdx` | **Still true, and it is not going to change.** `Custom` is settable over IPC and honoured by `EqPreset::gains()`, so it is a working feature with no TUI surface, not dead state. Document it as an IPC capability. |
| `playlist-dedup` "in practice can never remove anything — the primary key already prevents duplicates, so it is effectively a position repack" | `library.mdx` | **[fixed]** It now groups by `tracks.path`, which the `(playlist_id, track_id)` key does not cover — two different track ids on one file are possible, since `tracks.path` is indexed but not unique. |
| The `.m3u8` stub "is **not kept in sync** … never contains a track and is not a usable export" | `library.mdx` | **[fixed]** Every mutation rewrites the mirror through the same `M3u8Format` the manual exporter uses. It is now a real, playable file. |
| The help buffer "is a hand-maintained list … neither complete nor exact" | `tui.mdx` | *Still true.* It found two real bugs on its own — the duplicate `Alt+P` and the `Alt+H` with no binding — so it is worth keeping, just not worth quoting as complete. |
| "`check_health` has no default key. The help screen advertises `Alt+H`, but no such binding exists" | `keybindings.mdx` | **[fixed]** `Alt+H` is now bound to `KeyboardAction::CheckHealth`. |
| `Alt+p` "shadowed and unreachable by default" for Podcasts | `keybindings.mdx` | *Still true as a record of the old bug.* `Alt+p` is Podcasts and nothing else. |
| "note `Alt+p` is already mapped to the Progress Style picker" | `podcasts.mdx` | **[changed]** The premise is doubly obsolete: `Alt+p` is Podcasts, and there is no Progress Style picker to collide with — see [B3](#b3-progress-style-key-changed). |
| Keybinding contexts "are metadata, not live modes … never dispatched by the keymap at all" | `keybindings.mdx` | *Still true, and re-confirmed at `d433e51`.* The only production `dispatch` call passes `KeyContext::Normal`, so the `LIST` and `LIST_ONLY` bindings never fire. **This is not a dead-key bug**: arrows, `j`/`k`, `Shift+Up`/`Shift+Down`, `Alt+S` and the `Ctrl+j`/`Ctrl+k` queue moves are all handled directly in `gtm/src/app/keys.rs`, and the app even discards the four `QueueMove*` actions explicitly when they do arrive. The contexts are vestigial metadata for keys that have a hand-written path. Changing the dispatch to pass a real context would be a behavioural change with real regression risk and nothing observable to gain, so the code was left alone and the docs are right. |
| "There's a lot more involved in setting things up and I decided to just take the naive route because its enough, at least for now." | `benchmark.mdx` | *Still true.* |

See also [D](#d-bugs-found-in-gtmrs--all-fixed) — several of these had a root cause
in the code.

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

The docs section was removed. Re-confirmed at `d433e51`: still no `speed` or
`rate` symbol anywhere in `gtm/src` or `gtmd/src`, still no `CliCommand::Speed`,
still no field on `AudioSettings` and still no `Speed` variant on `FooterModule`.

**This is the one item in section B that is still an open decision, and it is a
product-scope question rather than a bug.** Either the feature is restored or the
scope is decided to exclude it; nothing in the code can settle that. Everything
else in section B is either fixed or already corrected on the docs side.

### B2. No visualizer toggle key

`Ctrl+v` is documented as "toggle the visualizer" in `keybindings.mdx` and
`audio.mdx`. There is no `Char('v') + CONTROL` binding in
`gtm/src/keymap.rs`. The visualizer now renders only in Zen mode and during
daydreaming (`gtm/src/ui/chrome.rs:238`).

**[fixed]** The `enabled` flag and the `Ctrl+v` binding were both removed rather
than the docs being edited around them: Zen or daydreaming decides whether the
visualizer draws, and a second switch could only disagree with it. Delete the
`Ctrl+v` rows; describe the two conditions that actually gate it.

### B3. Progress style key **[changed]**

The original finding was that progress style is `Alt+b`, not `Alt+p`, because
`Alt+p` opens Podcasts. That was fixed in `gtm/src/keymap.rs`, and the docs were
never updated.

**[changed]** It is now neither. The presentation picker replaced the five
separate overlays, so there is no Progress Style key at all:

- `Alt+L` opens the picker on Zen Layout.
- `Tab` / `Shift+Tab` moves between the five categories.
- `Left` / `Right` changes the value, which applies it immediately.
- `Alt+c` and `Alt+v` still open the picker directly on Theme and Visualizer.
- Progress has no key of its own: `Alt+b` is Browse, and giving it a shortcut
  would have re-created exactly the `Alt+p` bug this item is about.

Writing `Alt+b` into the docs now would be wrong again. Rewrite these as one
"Presentation" entry describing `Alt+L` plus `Tab`.

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

Re-measured at `d433e51`. Two of these have moved again since the audit, in the
opposite direction.

| Claim | Actual | Source |
|---|---|---|
| 14 library categories | 15 — `Browse` was added | `LIBRARY_CATEGORIES`, `gtm/src/app/mod.rs` |
| 14 `left_pane_lists` names | 15, and `Browse` is missing from the allowlist | `configuration.mdx`, `clean_left_pane` |
| 22 footer modules | **20**, and no `Speed` | `gtm/src/footer.rs` |
| 51 command-palette actions | **52**, not 54 | `gtm/src/ui/command.rs` |

The palette lost three rows (Theme, Progress Style, Visualizer Preset) and gained
one (Presentation), and `COMMAND_GROUPS` sums to 52. The 54 in the original audit
was the count *after* Browse was added but *before* the picker consolidation.

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

**Four of the eight have shipped since the audit and are now undocumented
features rather than candidate pages.** They are the ones to write first.

1. **Charts** — Spotify and Deezer Top Charts
   (`gtmd/src/providers/charts/`; Deezer landed in `9b0c5a7`). Spotify's
   `/v1/charts` is dead (410) and Apple Music needs a token, so Deezer is what
   actually answers. Today: one table row in `library.mdx` and two list mentions.
2. **Browse** — the 15th sidebar category, backed by Deezer: free-text search,
   artist pages, album tracklists (`gtmd/src/providers/browse.rs`). Shipped, and
   completely undocumented. It is also the reason Spotify artist/album browsing
   is absent: Spotify's public API no longer exposes artist contents.
3. **Notifications** — 10 categories (`NotifType::ALL`, `gtm/src/app/notify.rs`),
   each with a `floating` / `footer` / `off` mode, plus a history overlay and
   floating cards. The three `[extensions]` switches belong here.
4. **Daydreaming** — the visualizer takes over after `daydream_secs` of idle
   (default 60), set in Settings → System. Shipped.
5. **Discord Rich Presence** — `discord_id` in config, numeric-only validation
   (`gtmd/src/config.rs`), cleared on daemon quit, reachable via `Alt+x` →
   Discord (`PickerId::DiscordSetup`). Shipped; the presence is now cleared
   rather than left to expire.
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
keys with no documentation at all. `configuration.mdx` gained rows for both.

### Loudness mode and pre-gain have no surface at all

`LoudnessMode` (Off / Track / Album / Auto) and `pre_gain_db` are persisted in
`AudioSettings` and settable over IPC (`SetLoudnessMode`, `SetPreGain`), with
their own events — but there is no CLI command, no Settings row, and no
`keymap.rs` action. `audio.mdx` now has a short section saying exactly that.
Worth deciding whether they are a feature to expose or state to drop.

Re-confirmed at `d433e51`: nothing in `gtm/src/cli.rs`, `gtm/src/keymap.rs`,
`gtm/src/app/settings_keys.rs` or `settings_rows.rs` mentions either. Unlike
`gapless` these two *do* affect the signal chain, so dropping them would be a real
behaviour change; exposing them is the smaller of the two jobs and needs no new
machinery, only a Settings row. **Open decision, same shape as [B1](#b1-playback-speed-is-documented-on-six-pages-and-does-not-exist).**

### Packaging

`Formula/gtm.rb` (Homebrew) and `flake.nix` (Nix) exist in this repository but
were documented nowhere — `install.mdx` listed crates.io alone. Both added.

---

## D. Bugs found in gtm.rs — all fixed

Independent of the docs. Found while auditing to verify claims above. **All nine
were fixed**; the notes say how, because three of them changed what the docs are
allowed to claim.

1. **`gtm/src/ui/help.rs` advertised `Alt+P` twice** — once for Progress Style
   and once for Podcasts. The `?` buffer therefore told the user two different
   things about the same key. Fixed when the Progress Style row went away with
   the overlay it described.
2. **`Alt+H` was advertised for `check_health` with no binding behind it.** The
   action, the CLI command and the advertised key all existed; only the default
   binding was missing, so pressing the key the app told you about did nothing.
   **Bound it.**
3. **`i` (track info) and `l` (fetch lyrics) worked but appeared in no doc page.**
   Both were already in the in-app help; `docs/man/gtm.1.md` now has both.
4. **`queue-set --start-idx` was parsed, threaded through IPC, and discarded.**
   **Removed**, along with the client parameter, the IPC field and the dead
   daemon function. The queue model makes it ambiguous — index 0 is the
   currently-playing entry, so "start at N" either drops tracks or starts
   playback nobody asked for. The one caller that wanted a row was already
   working around it with an explicit `play`.
5. **`.m3u8` playlist stubs were written once and never updated.** They were a
   two-line header at create time, so the file was never a usable export despite
   the comment claiming one. **Every mutation rewrites it now**, through the
   same `M3u8Format` the manual exporter uses. Rename rewrites rather than moves,
   since the header carries the name.
6. **`playlist-dedup` could not remove anything.** It grouped by `track_id`,
   which the primary key already forbids. **It now groups by `tracks.path`**,
   which the key does not cover: `path` is indexed but not unique, so two
   different track ids can point at one file.
7. **`gapless` was persisted and never read.** **Removed entirely** — state
   field, FSM method, `SetGapless` request, `GaplessChanged` event and both
   man-page rows. An external IPC client sending `set_gapless` now gets an error
   instead of a silent no-op, which is the better failure. Existing `state.json`
   files keep the now-unknown key and serde ignores it.
8. **No WMA decoder, but `wma` was in the accepted extension list.** Confirmed:
   Symphonia's `all-codecs` is aac, adpcm, alac, flac, mp1, mp2, mp3, pcm and
   vorbis, and there is no WMA codec crate in the lockfile. **Removed**, with a
   separate "unsupported audio format" error — it *is* audio, and that
   distinction is the whole answer.
9. **`EqPreset::Custom([f32; 15])` was unreachable from the UI.** **Not removed,
   deliberately.** It is settable over `set_eq_preset` and honoured by
   `EqPreset::gains()`, so per-band EQ is a working IPC capability that simply
   has no TUI surface. Deleting it would have broken external clients to tidy up
   a documentation gap. Document it as what it is.