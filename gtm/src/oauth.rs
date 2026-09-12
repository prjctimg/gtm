// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// OAuth loopback capture helpers shared by the CLI wizard and the TUI setup
// flows (Last.fm token callback).
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default Last.fm loopback callback port.
pub const LASTFM_CALLBACK_PORT: u16 = 8991;

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
        if let Some((k, v)) = pair
            .split_once('=')
            .filter(|(k, v)| *k == name && !v.is_empty())
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Keep a credential short for display: first and last two characters only.
pub fn mask_credential(s: &str) -> String {
    if s.chars().count() <= 6 {
        "****".to_string()
    } else {
        format!("{}…{}", &s[..2], &s[s.len() - 2..])
    }
}

/// Bind the Last.fm callback port and wait (up to five minutes) for the
/// authorization redirect carrying a `token` query parameter. Responds 200 and
/// returns the token, or continues waiting on unrelated requests with a 404.
pub async fn capture_lastfm_token_loopback() -> Result<String, String> {
    let port = lastfm_callback_port();
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind the 127.0.0.1:{port} callback server: {e}"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
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
        let n = match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await {
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
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        let _ = stream.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_param_extracts_named_field() {
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
    fn mask_credential_hides_value() {
        assert_ne!(
            mask_credential("aVeryLongSecretValue"),
            "aVeryLongSecretValue"
        );
        assert_eq!(mask_credential("abc"), "****");
    }
}
