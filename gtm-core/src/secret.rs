// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// OS keychain-backed secret storage for gtm credentials (Spotify client id and
// OAuth token). Uses the `keyring` crate (secret-service on Linux, Keychain on
// macOS, Credential Manager on Windows). When no keychain backend is available
// (e.g. Termux / Android / headless containers) it transparently falls back to
// a per-user file under the config directory.
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

/// Keychain service name under which all gtm secrets are stored.
pub const SERVICE: &str = "gtm";
/// Keychain/username for the Spotify app client id.
pub const SPOTIFY_CLIENT_ID_KEY: &str = "spotify_client_id";
/// Keychain/username for the Spotify OAuth token.
pub const SPOTIFY_TOKEN_KEY: &str = "spotify_token";

fn fallback_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".config"))
            .unwrap_or_else(|| PathBuf::from(".config"))
    });
    base.join("gtm").join("secrets")
}

fn file_path(key: &str) -> PathBuf {
    fallback_dir().join(format!("{key}.secret"))
}

fn file_set(key: &str, value: &str) -> Result<(), String> {
    let p = file_path(key);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&p, value).map_err(|e| e.to_string())
}

fn file_get(key: &str) -> Result<Option<String>, String> {
    let p = file_path(key);
    match std::fs::read_to_string(&p) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn file_delete(key: &str) {
    let _ = std::fs::remove_file(file_path(key));
}

/// Store a secret. Tries the OS keychain first; if that's unavailable or
/// errors (no backend, locked collection with no agent, etc.) it falls back to
/// an on-disk file. Best-effort: failures are swallowed by callers.
pub fn set_secret(key: &str, value: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key)
        && entry.set_password(value).is_ok()
    {
        return;
    }
    let _ = file_set(key, value);
}

/// Load a secret. Returns `None` when it isn't stored. Keychain errors (e.g. a
/// locked collection) fall back to the on-disk file; callers that still get
/// `None` should prompt the user (masked) for the value.
pub fn get_secret(key: &str) -> Option<String> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key)
        && let Ok(p) = entry.get_password()
    {
        return Some(p);
    }
    file_get(key).ok().flatten()
}

/// Remove a stored secret from both the keychain and the file fallback.
pub fn delete_secret(key: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
        let _ = entry.delete_credential();
    }
    file_delete(key);
}
