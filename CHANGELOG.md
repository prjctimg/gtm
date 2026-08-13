# Changelog

## [0.1.5] – 2026-08-11

### Added
- **crates.io publishing**: workspace crates (`gtm-core`, `gtm-audio`, `gtm-mpris`, `gtmd`, `gtm`) are now published to crates.io by the release workflow (stable releases only, dependency order, idempotent re-runs via the `GTM_IO` token). Added missing `description` fields, explicit versions on path dependencies, and marked `release-gen` as `publish = false`
- **More distro packages**: release workflow now publishes `.rpm` (Fedora/RHEL/CentOS, x86_64 + aarch64, binary packaging via `dist/gtm-full.spec`), Arch `.pkg.tar.zst` (`scripts/build-arch-pkg.sh`), and Alpine `.apk` packages plus static musl archives (`scripts/build-alpine-apk.sh`, new `x86_64-linux-musl`/`aarch64-linux-musl` Alpine container jobs)
- **`checksums.txt`** with SHA-256 of every release asset

### Changed
- Release assets are staged per build job into `release-assets/` and collected flat by the release job, fixing the release glob that silently dropped the Linux/macOS archives on v0.1.3
- Per-target archives are now self-contained bundles (`bin/`, `man/man1/`, `completions/`, `systemd/`, `desktop/`, `icons/`, `LICENSE`); no separate TUI/daemon-only archives, raw binaries, or standalone `gtm-docs.tar.gz` are published — completions and man pages ship inside every archive
- README gains a "GitHub Release" install section (tar.xvf + per-distro one-liners)

### Fixed
- **Lyric auto-follow freeze**: manual lyric scrolling no longer leaves the highlight frozen for the rest of the track — auto-follow resumes once playback catches up to the scrolled line, and a tab switch clears the latch
- **Tab pane focus cycling**: Tab/Shift-Tab now cycles pane focus through the lyrics pane on the Library tab (previously only the library/settings panes), with focus state reset on tab switches
- **Active tab highlight**: the active tab now uses selection foreground/background colors in both the Classic and Modern designs
- Stale `dist/gtmd.spec` URL (`skchr/gtm-rs` → `prjctimg/gtm.rs`) and version

## [0.1.4] – 2026-08-09

### Fixed
- **Manpage generation in release pipeline**: the `docs/man` manpage sources (gtm.1, gtmd.1, gtmd-ipc.1) were removed while the gtm.spec repo still carries no `man/` tree, so `gen-manpages.sh` produced zero manpages and the release build failed. Restored the local `docs/man/*.1.md` sources and made the script fail with a clear diagnostic when no manpages can be generated
- **CI formatting gate**: `cargo fmt --check` was failing on the Design-toggle additions in `gtm/src/ui.rs`; reformatted

### Changed
- README docs section now links to the in-repo manpages (`docs/man/*.1.md`) instead of the wiki

## [0.1.3] – 2026-08-06

### Fixed
- **glibc compatibility**: Linux builds now run on Ubuntu 22.04 (glibc 2.35) instead of 24.04 (2.39), so binaries run on any system with glibc >= 2.35 (Debian 12 / Ubuntu 22.04 and later)
- **Broken `.deb` packages**: `gtm-full` debs had `Depends: libasound2t64, libc6 (>= 2.39)` which are unresolvable on Debian 12; deps are now resolved on the Ubuntu 22.04 build host (`libasound2`, `libc6 (>= 2.35)`). Also fixed the missing `gtm` binary and asset files that were packaged as dangling symlinks (cargo-deb 3.7.0 array-asset bug — switched to the `{source, dest}` table format and added `$auto`)
- **Decluttered releases**: per-platform `gtm-full-{platform}.tar.gz` now bundles both binaries + manpages + completions + systemd service; separate `gtm-*`/`gtmd-*` archives and raw binaries removed; all docs shipped as a single `gtm-docs.tar.gz`; the empty `gtmd` deb is no longer published

## [0.1.2] – 2026-08-05

### Added
- **Release pipeline fixes**: Termux builds now compile `termux-elf-cleaner` from source; Termux `.deb` built via `termux-create-package` manifest; TUI-only archives (`gtm-*.tar.gz`) published; `nightly` prerelease trigger (force-tagged `nightly`)
- **install.sh**: added `--deb`/`--type deb` path for Debian/Ubuntu and Termux; fixed version detection and latest-tag resolution (prerelease-aware)
- **Wiki**: moved to a dedicated wiki repository (https://github.com/prjctimg/gtm.rs/wiki)

## [0.1.0] – 2025-07-10

### Added
- **Tab-based TUI**: Now Playing, Library, Settings tabs with Tab/Shift+Tab navigation
- **Library browser**: left-pane categories (All Tracks, Playlists, Favourites, Recent) with counts, right-pane track listing
- **Settings panel**: volume, crossfade, repeat, shuffle, sleep timer, about, overlays
- **Cover art display**: embedded album art extracted via symphonia, cached as PNG, rendered as pixelated blocks
- **Theme system**: 7 presets (Default, Catppuccin, Nord, Dracula, Solarized, Monokai, Gruvbox) via `AppTheme` struct
- **Toast notifications**: volume/mute/repeat/shuffle changes shown as auto-expiring 1-line toasts
- **Enhanced footer**: status section (play/pause icon, volume gauge, repeat/shuffle indicators), inline progress bar (20-char + elapsed/total), keyboard hints
- **Notification overlay**: sleep timer countdown in Now Playing tab
- **Crossfade audio**: smoothstep-eased crossfade transitions between tracks
- **Volume dip on pause**: 150ms linear fade to silence, instant resume
- **CLI mode**: 30+ subcommands for playback, queue, library, YouTube, and daemon control
- **IPC protocol**: mixed JSON+binary framing over Unix socket
- **Packaging**: DEB (cargo-deb), RPM spec, Nix flake, Makefile, systemd user service, desktop file

### Fixed
- `blocking_lock()` crash in IPC worker — switched to async `.lock().await`
- 10-second blank startup — background auto-scan, non-blocking socket accept
- Unsafe volume clamp on daemon startup — explicit `min(100)`
- TUI freeze on library scan — deferred scanning to tokio task
- Enter key in Library tab now plays selected track
- Position tracking freezes correctly on pause

### Changed
- Ported from Nim to Rust (gtm-rs)
- Architecture: client/daemon split over Unix socket IPC
- Audio backend: rodio + symphonia (was GStreamer)
- State management: state machine with DaemonState, PlaybackStatus
- Removed dead tabs: Queue, YouTube, Help (functionality merged or dropped)
