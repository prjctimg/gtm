# Changelog

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
