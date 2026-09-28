// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// OAuth browser launch and loopback capture helpers shared by the CLI wizard
// and the TUI setup flows.
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default Last.fm loopback callback port.
pub const LASTFM_CALLBACK_PORT: u16 = 8991;

/// Best-effort browser open for an OAuth authorize URL. Tries the OS default
/// opener (via `webbrowser`) then common launchers, each timeout-guarded so a
/// wedged launcher never blocks a runtime worker or the UI.
pub async fn open_browser(url: &str) -> bool {
    // Prefer the OS default browser opener, which is cross-platform. Guard
    // it with a timeout: a wedged `xdg-open`-style launcher must not leave
    // the flow looking dead.
    let opened = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::task::spawn_blocking({
            let url = url.to_string();
            move || webbrowser::open(&url)
        })
        .await
    })
    .await;
    if let Ok(Ok(Ok(_))) = opened {
        return true;
    }
    // Fallback to common launchers when the `webbrowser` crate can't
    // resolve one (e.g. minimal containers / WSL).
    for prog in ["xdg-open", "open", "start"] {
        match tokio::process::Command::new(prog)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
        {
            Ok(st) if st.success() => return true,
            _ => continue,
        }
    }
    false
}

/// Callback port resolved from `$GTM_LASTFM_PORT`, falling back to
/// [`LASTFM_CALLBACK_PORT`].
pub fn lastfm_callback_port() -> u16 {
    std::env::var("GTM_LASTFM_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LASTFM_CALLBACK_PORT)
}

/// Extract a named query parameter from the first line of an HTTP request, a
/// bare path, or a URL, e.g. `"/login?token=abc&api_key=k"`.
pub fn query_param(line: &str, name: &str) -> Option<String> {
    let query = line
        .split_whitespace()
        .find_map(|tok| tok.split_once('?').map(|(_, q)| q))?;
    for pair in query.split('&') {
        if let Some((_k, v)) = pair
            .split_once('=')
            .filter(|(k, v)| *k == name && !v.is_empty())
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Keep a credential short for display: first and last two characters only.
///
/// Char-indexed, not byte-indexed. This used to only ever see hex session keys,
/// where byte and char offsets coincide, so `&s[..2]` was safe by accident. It
/// is now also fed whatever a user pastes into the Spotify client-id field, and
/// a smart quote or an em-dash puts byte offset 2 in the middle of a character —
/// which panics on a char boundary. Pasted text is arbitrary, so the slicing
/// has to be.
pub fn mask_credential(s: &str) -> String {
    if s.chars().count() <= 6 {
        return "****".to_string();
    }
    let head: String = s.chars().take(2).collect();
    // Two from the end, without indexing backwards: there is no stable way to
    // take a tail slice without first knowing the char count.
    let tail: String = s
        .chars()
        .rev()
        .take(2)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// Bind the Last.fm callback port and wait (up to five minutes) for the
/// authorization redirect carrying a `token` query parameter. Responds 200 and
/// returns the token, or continues waiting on unrelated requests with a 404.
pub async fn capture_lastfm_token() -> Result<String, String> {
    bind_lastfm_callback().await?.wait_for_token().await
}

/// A pre-bound Last.fm loopback callback server. Bind it *before* opening the
/// browser so the authorization redirect never hits a dead port, then wait.
pub struct LastfmCallback {
    listener: tokio::net::TcpListener,
    addr: String,
    deadline: tokio::time::Instant,
}

/// Bind the Last.fm callback port (five-minute wait deadline). The caller
/// should open the browser only after this succeeds.
pub async fn bind_lastfm_callback() -> Result<LastfmCallback, String> {
    let port = lastfm_callback_port();
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind the 127.0.0.1:{port} callback server: {e}"))?;
    Ok(LastfmCallback {
        listener,
        addr,
        deadline: tokio::time::Instant::now() + Duration::from_secs(300),
    })
}

impl LastfmCallback {
    pub async fn wait_for_token(self) -> Result<String, String> {
        let LastfmCallback {
            listener,
            addr,
            deadline,
        } = self;
        loop {
            let accepted = tokio::time::timeout(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                listener.accept(),
            )
            .await;
            let (mut stream, _) = match accepted {
                Err(_) => {
                    return Err(format!(
                        "timed out waiting for the callback on http://{addr}"
                    ));
                }
                Ok(Err(e)) => return Err(format!("callback accept: {e}")),
                Ok(Ok(pair)) => pair,
            };
            let mut buf = [0u8; 4096];
            let n = match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await
            {
                Ok(Ok(n)) => n,
                _ => 0,
            };
            let line = String::from_utf8_lossy(&buf[..n]).to_string();
            if let Some(token) = query_param(&line, "token")
                && !token.is_empty()
            {
                let body = "gtm authorized. You can close this tab.";
                let _ = stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
                let _ = stream.flush().await;
                return Ok(token);
            }
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            let _ = stream.flush().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_field() {
        assert_eq!(
            query_param("GET /?token=abc123&api_key=k2 HTTP/1.1", "token"),
            Some("abc123".to_string())
        );
        assert_eq!(
            query_param("GET /lastfm?api_key=k2&token=xyz HTTP/1.1", "token"),
            Some("xyz".to_string())
        );
        assert_eq!(
            query_param("http://127.0.0.1:8991/lastfm?token=qwe", "token"),
            Some("qwe".to_string())
        );
        assert_eq!(query_param("GET / HTTP/1.1", "token"), None);
        assert_eq!(query_param("GET /?code=abc HTTP/1.1", "token"), None);
        assert_eq!(query_param("", "token"), None);
    }

    #[test]
    fn mask_hides_value() {
        assert_ne!(
            mask_credential("aVeryLongSecretValue"),
            "aVeryLongSecretValue"
        );
        assert_eq!(mask_credential("abc"), "****");
        assert_eq!(mask_credential("0123456789abcdef"), "01…ef");
    }

    /// Pasted text is arbitrary, and the Spotify client-id field runs it
    /// through this on every frame. A byte-indexed mask panics on the first
    /// multi-byte character past offset 2, which a pasted smart quote or em
    /// dash reaches immediately — so these must not panic.
    #[test]
    fn mask_survives_multibyte_input() {
        // `a€…` — byte 2 lands inside the 3-byte euro sign.
        assert_eq!(mask_credential("a€bcd€fgh"), "a€…gh");
        // Leading multi-byte, where the first two bytes are not even one char.
        assert_eq!(mask_credential("€uroclientid"), "€u…id");
        // Longer than 6 chars but few bytes of overlap between head and tail.
        assert_eq!(mask_credential("€€€€€€€"), "€€…€€");
        // Short multi-byte strings still take the **** path, by char count.
        assert_eq!(mask_credential("€€€"), "****");
        // Empty and whitespace are total, not partial.
        assert_eq!(mask_credential(""), "****");
    }
}
