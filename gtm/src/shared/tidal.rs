// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Tidal integration: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// Default local redirect port for the Tidal OAuth flow. The loopback port is
/// never auto-selected from a built-in client id: Tidal's community "web"
/// client id (`CzET4vdadNUFQ5JU`) redirects to `listen.tidal.com/login/auth`,
/// NOT a loopback URI, so the link flow requires a user-registered client id
/// that lists `http://127.0.0.1:{port}/login` as a Redirect URI.
pub const TIDAL_DEFAULT_PORT: u16 = 8992;

/// Connection state of the Tidal integration, surfaced in the Setup picker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TidalStatus {
    /// Whether a token is configured and the link is live.
    pub linked: bool,
    /// Link failure surfaced through the status poll (callback timeout,
    /// token-exchange error, …). `None` while idle or linked.
    pub error: Option<String>,
}
