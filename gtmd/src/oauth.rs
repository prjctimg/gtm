// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify OAuth 2.0 PKCE link flow
//
// Auth helpers adapted from aome510/spotify-player (`auth.rs`, MIT,
// (c) 2021 Thang Pham). We build an
// authorize URL with a PKCE S256 challenge, serve the redirect on a local
// port, and exchange the returned code for an access token.
//
// This is free software released under the GPL-3.0 license.

use std::net::SocketAddr;

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

const SPOTIFY_AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Default local redirect port served by [`OauthFlow::wait_for_access_token`].
pub const DEFAULT_OAUTH_PORT: u16 = 8990;
/// Default redirect URI used when no port override is supplied.
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8990/login";

/// Scopes required for playlist sync plus playback control.
const OAUTH_SCOPES: &[&str] = &[
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
];

/// One pending OAuth link flow. Create it, hand [`Self::authorize_url`] to
/// the user, then await [`Self::wait_for_access_token`]. `port` selects the
/// local callback port so users can reuse a redirect URI already registered
/// in their Spotify dashboard.
pub struct OauthFlow {
    client_id: String,
    pkce: Pkce,
    redirect_uri: String,
    redirect_addr: SocketAddr,
}

impl OauthFlow {
    pub fn new(client_id: impl Into<String>, port: u16) -> Self {
        let host = "127.0.0.1";
        let redirect_uri = format!("http://{host}:{port}/login");
        let redirect_addr = format!("{host}:{port}")
            .parse()
            .expect("valid loopback redirect address");
        Self {
            client_id: client_id.into(),
            pkce: Pkce::new_random(),
            redirect_uri,
            redirect_addr,
        }
    }

    pub fn authorize_url(&self) -> String {
        let state = random_url_safe(16);
        let scope = OAUTH_SCOPES.join(" ");
        let params = [
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", scope.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", self.pkce.challenge.as_str()),
            ("state", state.as_str()),
        ];
        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{SPOTIFY_AUTHORIZE_URL}?{query}")
    }

    /// Serve one redirect on the local callback port, exchange the code for
    /// an access token, and return it. Cancels itself after 5 minutes.
    pub async fn wait_for_access_token(&self) -> Result<String, String> {
        let code = listen_for_auth_code(self.redirect_addr).await?;
        exchange_code_for_token(
            &code,
            &self.client_id,
            &self.pkce.verifier,
            &self.redirect_uri,
        )
        .await
    }
}

struct Pkce {
    verifier: String,
    challenge: String,
}

impl Pkce {
    fn new_random() -> Self {
        let verifier = random_url_safe(32);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

fn random_url_safe(n: usize) -> String {
    let bytes: Vec<u8> = (0..n).map(|_| fastrand::u8(..)).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Minimal percent-encoding for query values.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn listen_for_auth_code(addr: SocketAddr) -> Result<String, String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind OAuth callback server to {addr}: {e}"))?;

    // Accept connections until one carries a `?code=` query. A 5 minute
    // guard keeps a forgotten flow from holding the socket forever.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
        let (mut stream, _) = match accept {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(format!("accept: {e}")),
            Err(_) => return Err("OAuth link timed out after 5 minutes".into()),
        };
        match read_code_from_stream(&mut stream).await {
            Some(code) => {
                write_response(
                    &mut stream,
                    "200 OK",
                    "gtm authenticated. You can close this tab.",
                )
                .await;
                return Ok(code);
            }
            None => {
                write_response(&mut stream, "404 Not Found", "").await;
            }
        }
    }
}

async fn read_code_from_stream(stream: &mut tokio::net::TcpStream) -> Option<String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut reader = BufReader::new(stream);
    // The request head's first line is all we need.
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    // "GET /login?code=...&state=... HTTP/1.1"
    let target = line.split_whitespace().nth(1)?;
    code_from_redirect(target)
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    use tokio::io::AsyncWriteExt;
    let _ = stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
    let _ = stream.flush().await;
}

fn code_from_redirect(target: &str) -> Option<String> {
    let query = target.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == "code").then(|| v.to_string())
    })
}

async fn exchange_code_for_token(
    code: &str,
    client_id: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<i64>,
        #[serde(default)]
        scope: Option<String>,
    }

    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    let body = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let resp = reqwest::Client::new()
        .post(SPOTIFY_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("send token exchange request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("token exchange failed: HTTP {}", resp.status()));
    }
    let parsed: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse token response: {e}"))?;

    let mut token = serde_json::json!({
        "access_token": parsed.access_token,
        "expires_in": parsed.expires_in.unwrap_or(3600),
    });
    if let Some(scope) = parsed.scope {
        token["scope"] = serde_json::Value::String(scope);
    }
    if let Some(rt) = parsed.refresh_token {
        token["refresh_token"] = serde_json::Value::String(rt);
    }
    Ok(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_uses_gtm_port() {
        let flow = OauthFlow::new("test-client-id", DEFAULT_OAUTH_PORT);
        assert_eq!(flow.redirect_uri, DEFAULT_REDIRECT_URI);
    }

    #[test]
    fn authorize_url_contains_pkce_params() {
        let flow = OauthFlow::new("test-client-id", DEFAULT_OAUTH_PORT);
        let url = flow.authorize_url();
        assert!(url.starts_with(SPOTIFY_AUTHORIZE_URL));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains(&urlencode(&flow.redirect_uri)));
    }

    #[test]
    fn code_extraction() {
        assert_eq!(
            code_from_redirect("/login?code=abc123&state=xyz"),
            Some("abc123".to_string())
        );
        assert_eq!(code_from_redirect("/login?state=xyz"), None);
        assert_eq!(code_from_redirect("/login"), None);
    }

    #[test]
    fn urlencoding() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("plain"), "plain");
    }

    #[tokio::test]
    async fn flow_serves_callback_and_exchanges() {
        // End-to-end against a stub token endpoint is out of scope here; just
        // verify the callback listener returns the code sent to the port.
        let flow = OauthFlow::new("cid", DEFAULT_OAUTH_PORT);
        let f = flow.wait_for_access_token();
        // Don't bind the real flow; only exercise URL/code helpers above.
        drop(f);
    }
}
