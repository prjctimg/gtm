# 13 — Development Phases

## Phase 1 — Bug Fixes (Current)

**Goal:** Make the existing code functional and stable.

| # | Bug | Fix | Files |
|---|-----|-----|-------|
| 1 | Cold start "Connection refused" | Retry loop with ping in `ensure_daemon_running` + retry in `DaemonClient::connect` | `gtm/src/ui.rs`, `gtm-core/src/client.rs` |
| 2 | Screen not clearing | Add `terminal.clear()?` after `Terminal::new()` | `gtm/src/ui.rs` |
| 3 | Audio files not detected | Fix hardcoded `duration = 0.0` in `extract_metadata`; add "opus" to extensions; configurable library paths | `gtmd/src/library.rs`, `gtmd/src/config.rs`, `gtmd/src/daemon.rs` |
| 4 | Playback freezes TUI | Non-blocking IPC via background command processing; `Arc<Mutex<DaemonClient>>` | `gtm/src/app.rs`, `gtm-core/src/client.rs` |

**Checklist:**
- [ ] `gtm` cold start works without "Connection refused"
- [ ] TUI screen is clear on startup (no residual terminal text)
- [ ] Audio files in `~/.local/share/gtm/audio/` and configured paths are detected
- [ ] Track durations are non-zero in library
- [ ] Playback does not freeze the TUI
- [ ] `cargo check --workspace` passes
- [ ] No regressions in CLI mode

---

## Phase 2 — IPC Protocol Redesign

**Goal:** Non-blocking IPC, snake_case commands, dedicated pulse channel.

| Step | Files | Change |
|------|-------|--------|
| 2.1 | `gtm-core/src/ipc.rs` | Add `#[serde(rename_all = "snake_case")]` to all IPC enums |
| 2.2 | `gtm-core/src/client.rs` | Rewrite `DaemonClient` with background worker task + channels |
| 2.3 | `gtmd/src/daemon.rs` | Add pulse socket (dedicated binary event broadcast) |
| 2.4 | `gtm/src/app.rs` | Switch to new non-blocking DaemonClient |
| 2.5 | `gtm/src/ui.rs` | Remove ping hack, clean up ensure_daemon_running |

**Checklist:**
- [ ] All IPC commands use snake_case
- [ ] `DaemonClient::send()` does not block the caller
- [ ] Events arrive via dedicated pulse socket
- [ ] No first-byte heuristic parsing
- [ ] `cargo test --workspace` passes

---

## Phase 3 — UI Architecture Overhaul

**Goal:** 3-tab layout, 9 overlays, aesthetic improvements.

| Step | Files | Change |
|------|-------|--------|
| 3.1 | `gtm-core/src/state.rs` | Replace `Tab` enum: only NowPlaying, Library, Settings |
| 3.2 | `gtm/src/ui.rs` | New layout: 3 tabs + overlay container layer |
| 3.3 | `gtm/src/overlay.rs` | Generic `OverlayContainer<T>` with fuzzy finder + keymap |
| 3.4 | `gtm/src/tabs/` | Tab implementations: `now_playing.rs`, `library.rs`, `settings.rs` |
| 3.5 | `gtm/src/overlays/` | Overlay implementations (9 overlays) |
| 3.6 | `gtm/src/ui.rs` | Rounded borders, braille spinners, icons |
| 3.7 | `gtm/src/progress.rs` | Line progress bar with oscillating head |
| 3.8 | `gtm/src/footer.rs` | Customizable footer modules + presets |
| 3.9 | `gtm/src/notifications.rs` | Up Next toast, volume toast with animations |

**Checklist:**
- [ ] Only 3 tabs: NowPlaying, Library, Settings
- [ ] All 9 overlays functional via Alt+key
- [ ] Rounded borders everywhere
- [ ] Braille loading spinners
- [ ] Nerd icons with emoji fallback
- [ ] Line progress bar with oscillating head
- [ ] Semi-transparent overlays (90% default)
- [ ] Footer with customizable presets
- [ ] Up Next notification on crossfade
- [ ] Volume toast with color levels

---

## Phase 4 — Audio & Crossfade Features

**Goal:** Professional crossfade with easing, reverb, volume dip, safe volume.

| Step | Files | Change |
|------|-------|--------|
| 4.1 | `gtm-audio/src/mixer.rs` | Add easing functions (`mixer.rs`) |
| 4.2 | `gtm-audio/src/mixer.rs` | Add reverb pipeline for transitions |
| 4.3 | `gtm-audio/src/mixer.rs` | Volume ramp on pause/resume |
| 4.4 | `gtm-core/src/ipc.rs` | Add `VolumeChallenge` / `ConfirmVolume` |
| 4.5 | `gtmd/src/daemon.rs` | Session ID management, volume challenge logic |
| 4.6 | `gtmd/src/daemon.rs` | Default crossfade 7s, easing config |
| 4.7 | `gtm/src/app.rs` | Volume challenge prompt handling |

**Checklist:**
- [ ] Crossfade default duration is 7s
- [ ] 5 easing options selectable
- [ ] Reverb on transitions (toggleable)
- [ ] Volume dips smoothly on pause, ramps on resume
- [ ] Volume >85% shows warning, requires confirmation
- [ ] Session ID securely shared between client and daemon
- [ ] No buffer underflow/overflow at end of queue

---

## Phase 5 — Feature Compliance & Polish

**Goal:** Match all features documented in legacy man pages.

| Step | Files | Change |
|------|-------|--------|
| 5.1 | `docs-legacy/gtm.1.md` | Audit all CLI flags, subcommands, behaviors |
| 5.2 | `docs-legacy/gtmd.1.md` | Audit all daemon flags, signals, features |
| 5.3 | `docs-legacy/gtmd-ipc.md` | Audit all IPC commands, responses, events |
| 5.4 | `gtm/src/cli.rs` | Add missing flags/subcommands |
| 5.5 | `gtm-core/src/ipc.rs` | Add missing IPC commands/events |
| 5.6 | `gtmd/src/daemon.rs` | Add missing daemon features |
| 5.7 | `Cargo.toml` | Update license, author metadata |

**Checklist:**
- [ ] All `gtm.1` flags and subcommands implemented
- [ ] All `gtmd.1` flags and features implemented
- [ ] All `gtmd-ipc.md` commands, responses, events implemented
- [ ] License and author metadata correct
- [ ] `gtm --help` matches legacy output
- [ ] `gtmd --help` matches legacy output

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| Phase 2 IPC redesign breaks Phase 1 fixes | High | Phase 1 keeps existing protocol; Phase 2 is a coordinated atomic change |
| Audio backend (symphonia) can't extract duration | Medium | Fallback to ffprobe subprocess |
| Pulse socket adds complexity | Medium | Single extra file descriptor, well-tested pattern |
| Nerd font detection unreliable | Low | Emoji fallback always available |
| Crossfade reverb too CPU-intensive | Low | Toggleable, short IR only |
