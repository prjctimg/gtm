# Changelog

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
