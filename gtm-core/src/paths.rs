// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Shared path resolution: Termux-aware socket and directory helpers
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

/// Detect whether we are running inside Termux.
///
/// Termux sets `$PREFIX` (always) and `$TERMUX_VERSION` (on newer versions).
fn is_termux() -> bool {
    std::env::var("PREFIX").is_ok() || std::env::var("TERMUX_VERSION").is_ok()
}

/// Return the default daemon socket path, resolved from the environment.
///
/// Resolution order:
/// 1. `$XDG_RUNTIME_DIR/gtmd.socket`
/// 2. `$TMPDIR/gtmd.socket`
/// 3. Termux fallback: `$PREFIX/tmp/gtmd.socket`
/// 4. `std::env::temp_dir().join("gtmd.socket")`
/// 5. Last resort: `$HOME/.gtm/gtmd.socket`
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(runtime).join("gtmd.socket");
        if p.parent().map_or(false, |d| d.exists()) {
            return p;
        }
    }
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        let p = PathBuf::from(tmpdir).join("gtmd.socket");
        if p.parent().map_or(false, |d| d.exists()) {
            return p;
        }
    }
    if is_termux() {
        if let Ok(prefix) = std::env::var("PREFIX") {
            let p = PathBuf::from(prefix).join("tmp").join("gtmd.socket");
            if p.parent().map_or(false, |d| d.exists()) {
                return p;
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(home).join(".gtm").join("gtmd.socket");
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            return p;
        }
    }
    let tmp = std::env::temp_dir().join("gtmd.socket");
    if tmp.parent().and_then(|p| p.to_str()).unwrap_or("") != "/tmp" {
        return tmp;
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".gtm").join("gtmd.socket");
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        return p;
    }
    tmp
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
