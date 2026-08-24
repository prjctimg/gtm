// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Shared path resolution: Termux-aware socket and directory helpers
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

/// Detect whether we are running inside Termux.
///
/// Termux sets `$PREFIX` (always) and `$TERMUX_VERSION` (on newer versions).
pub fn is_termux() -> bool {
    std::env::var("PREFIX").is_ok() || std::env::var("TERMUX_VERSION").is_ok()
}

/// Return the state directory for runtime files (sockets, PID).
///
/// Resolution order:
/// 1. `$XDG_RUNTIME_DIR/gtm/`
/// 2. `/tmp/gtm-$USER/gtm/`
/// 3. `$TMPDIR/gtm/`
/// 4. Termux fallback: `$PREFIX/tmp/gtm/`
/// 5. `$HOME/.gtm/gtm/`
fn state_dir() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(&runtime).join("gtm");
        if p.parent().is_some_and(|d| d.exists()) {
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    if let Ok(user) = std::env::var("USER") {
        let p = PathBuf::from("/tmp")
            .join(format!("gtm-{}", user))
            .join("gtm");
        if let Some(parent) = p.parent() {
            if parent.exists() {
                let _ = std::fs::create_dir_all(&p);
                return p;
            }
        }
    }
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        let p = PathBuf::from(&tmpdir).join("gtm");
        if p.parent().is_some_and(|d| d.exists()) {
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    if is_termux() {
        if let Ok(prefix) = std::env::var("PREFIX") {
            let p = PathBuf::from(prefix).join("tmp").join("gtm");
            if p.parent().is_some_and(|d| d.exists()) {
                let _ = std::fs::create_dir_all(&p);
                return p;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(&home).join(".gtm").join("gtm");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    let p = std::env::temp_dir().join("gtm");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Return the default daemon command socket path.
///
/// Resolution order:
/// 1. `$XDG_RUNTIME_DIR/gtm/gtmd.sock`
/// 2. `/tmp/gtm-$USER/gtm/gtmd.sock`
/// 3. `$TMPDIR/gtm/gtmd.sock`
/// 4. Termux fallback: `$PREFIX/tmp/gtm/gtmd.sock`
/// 5. `$HOME/.gtm/gtm/gtmd.sock`
pub fn resolve_command_socket() -> PathBuf {
    state_dir().join("gtmd.sock")
}

/// Return the default daemon pulse socket path.
pub fn resolve_pulse_socket() -> PathBuf {
    state_dir().join("gtmd.pulse")
}

/// Return the default daemon PID file path.
pub fn resolve_pid_file() -> PathBuf {
    state_dir().join("gtmd.pid")
}

/// Return Termux-specific music library paths (if any exist).
///
/// On Termux this includes `/sdcard/Music` (via `$HOME/storage/shared/Music`)
/// when the user has run `termux-setup-storage`. Returns an empty `Vec` on
/// non-Termux platforms.
pub fn termux_music_dirs() -> Vec<PathBuf> {
    if !is_termux() {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let shared_music = PathBuf::from(&home)
            .join("storage")
            .join("shared")
            .join("Music");
        if shared_music.exists() {
            dirs.push(shared_music);
        }
    }
    let sdcard = PathBuf::from("/sdcard/Music");
    if sdcard.exists() && !dirs.iter().any(|d| d == &sdcard) {
        dirs.push(sdcard);
    }
    dirs
}
