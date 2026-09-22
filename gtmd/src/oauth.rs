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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SPOTIFY_AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// Default local redirect port served by [`OauthFlow::listen`].
pub const DEFAULT_OAUTH_PORT: u16 = 8990;

/// How long an OAuth link flow waits for the browser redirect before giving
/// up. Kept short enough that a forgotten flow fails fast and the Settings UI
/// reports it, instead of stalling silently for minutes.
pub const OAUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Default redirect URI used when no port override is supplied.
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8990/login";

/// Scopes required for playlist sync plus playback control. The user-profile
/// scopes let us read the display name/product. Note there is deliberately NO
/// `offline_access` here: Spotify does not define that scope (it rejected the
/// whole authorize request with `error=invalid_scope`), and Spotify's
/// authorization-code flow grants a `refresh_token` regardless, so tokens can
/// still be renewed after `expires_in`.
const OAUTH_SCOPES: &[&str] = &[
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
    "user-read-private",
    "user-read-email",
];

/// One pending OAuth link flow. Create it, bind the callback port with
/// [`Self::listen`], hand [`Self::authorize_url`] to the user (only after the
/// bind succeeds), then await [`Self::wait_token`]. `port` selects the local
/// callback port so users can reuse a redirect URI already registered in their
/// Spotify dashboard.
pub struct OauthFlow {
    client_id: String,
    pkce: Pkce,
    redirect_uri: String,
    redirect_addr: SocketAddr,
    /// Anti-CSRF token: generated once, embedded in the authorize URL, and
    /// verified against the value echoed back in the redirect.
    state: String,
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
            state: random_url_safe(16),
        }
    }

    pub fn authorize_url(&self) -> String {
        let state = self.state.clone();
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
    /// an access token, and return it. Cancels itself after [`OAUTH_TIMEOUT`].
    /// The redirect's `state` must match the value this flow issued, or the
    /// flow is rejected as a cross-site request forgery attempt. The listener
    /// must come from [`Self::listen`], which binds eagerly so the browser is
    /// only ever pointed at a live callback server.
    pub async fn wait_token(&self, listener: OauthListener) -> Result<String, String> {
        let code = listener.accept_token(&self.state).await?;
        exchange_code(
            &code,
            &self.client_id,
            &self.pkce.verifier,
            &self.redirect_uri,
        )
        .await
    }

    /// Bind the loopback callback socket now. Returning the authorize URL (or
    /// telling the caller the browser can be opened) should always happen
    /// *after* this succeeds; otherwise a fast user can land on a dead port.
    pub async fn listen(&self) -> Result<OauthListener, String> {
        let listener = tokio::net::TcpListener::bind(self.redirect_addr)
            .await
            .map_err(|e| format!("bind OAuth callback server to {}: {e}", self.redirect_addr))?;
        Ok(OauthListener {
            listener,
            deadline: tokio::time::Instant::now() + OAUTH_TIMEOUT,
        })
    }
}

/// A pre-bound loopback callback listener for a single OAuth link flow.
pub struct OauthListener {
    listener: tokio::net::TcpListener,
    deadline: tokio::time::Instant,
}

impl OauthListener {
    /// Accept connections until one carries a `?code=` query with a matching
    /// `state`, or the deadline elapses so a forgotten flow does not hold the
    /// socket forever. Spotify-specific thin wrapper over [`Self::accept_param`].
    async fn accept_token(self, expected_state: &str) -> Result<String, String> {
        self.accept_param("code", Some(expected_state)).await
    }

    /// Accept connections until one carries the named query `param` (with a
    /// matching `state` when one is expected), or the deadline elapses. The
    /// 200 response is written *before* the value is returned, so the browser
    /// always sees a live callback even if the daemon stalls afterwards.
    /// Unrelated requests (favicons, prefetches) get a 404 and the loop keeps
    /// waiting. This is the single callback contract every provider shares:
    /// bind first, open the browser second, and complete the flow inside the
    /// daemon instead of in the TUI's event loop.
    pub async fn accept_param(
        self,
        param: &str,
        expected_state: Option<&str>,
    ) -> Result<String, String> {
        let OauthListener { listener, deadline } = self;
        loop {
            let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
            let (mut stream, _) = match accept {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => return Err(format!("accept: {e}")),
                Err(_) => return Err("OAuth link timed out waiting for the browser".into()),
            };
            match read_param(&mut stream, param, expected_state).await {
                Ok(Some(value)) => {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "gtm authenticated. You can close this tab.",
                    )
                    .await;
                    return Ok(value);
                }
                Ok(None) => {
                    write_response(&mut stream, "404 Not Found", "").await;
                }
                Err(msg) => {
                    // The provider rejected the flow (e.g. `error=invalid_scope`);
                    // tell the user in the browser tab and fail the link
                    // immediately instead of silently waiting out the timeout.
                    write_response(
                        &mut stream,
                        "400 Bad Request",
                        &format!("OAuth authorization failed: {msg}"),
                    )
                    .await;
                    return Err(msg);
                }
            }
        }
    }
}

/// Bind a loopback callback listener for a provider flow. Binding succeeds
/// *before* any authorize URL is handed out or the browser is opened, so a
/// fast user never lands on a dead port. `timeout` bounds how long the
/// returned listener waits for the browser redirect.
pub async fn bind_callback(
    port: u16,
    timeout: std::time::Duration,
) -> Result<OauthListener, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind OAuth callback server to {addr}: {e}"))?;
    Ok(OauthListener {
        listener,
        deadline: tokio::time::Instant::now() + timeout,
    })
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
    let mut bytes = vec![0u8; n];
    getrandom::getrandom(&mut bytes).expect("OS random source available");
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

async fn read_param(
    stream: &mut tokio::net::TcpStream,
    param: &str,
    expected_state: Option<&str>,
) -> Result<Option<String>, String> {
    let mut reader = BufReader::new(stream);
    // The request head's first line is all we need.
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("read OAuth callback request: {e}"))?;
    // "GET /login?code=...&state=... HTTP/1.1"
    let target = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "malformed OAuth callback request".to_string())?;
    param_from_redirect(target, param, expected_state)
}

/// Extract `param` from a redirect target. When `expected_state` is given, the
/// redirect must also carry a matching `state` query value; a mismatch is a
/// CSRF signal and is rejected by returning `Ok(None)` (the acceptor keeps
/// waiting for a genuine redirect).
///
/// Returns `Err` when the redirect carries an `error`/`error_description`
/// parameter (e.g. `error=invalid_scope`), so the failure surfaces immediately
/// instead of silently 404-ing until the flow times out.
fn param_from_redirect(
    target: &str,
    param: &str,
    expected_state: Option<&str>,
) -> Result<Option<String>, String> {
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
    let mut value = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            k if k == param => value = Some(percent_decode(v)),
            "state" => state = Some(v.to_string()),
            "error" => error = Some(percent_decode(v)),
            "error_description" => error_description = Some(percent_decode(v)),
            _ => {}
        }
    }
    if let Some(kind) = error {
        let detail = error_description
            .filter(|d| !d.is_empty())
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        return Err(format!("{kind}{detail}"));
    }
    match expected_state {
        None => Ok(value),
        Some(expected) => {
            // Constant-time-ish comparison is not required for list equality
            // here; exact string equality is sufficient to bind the value to
            // our flow.
            if state.as_deref() == Some(expected) {
                Ok(value)
            } else {
                Ok(None)
            }
        }
    }
}

/// Back-compat wrapper over [`param_from_redirect`] for the Spotify `code`.
/// Retained for the unit tests; production paths go through [`read_param`].
#[cfg(test)]
fn code_from_redirect(target: &str, expected_state: &str) -> Result<Option<String>, String> {
    param_from_redirect(target, "code", Some(expected_state))
}

async fn write_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let _ = stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await;
    let _ = stream.flush().await;
}

/// Decode `%XX` escapes in a query value (Spotify error descriptions contain
/// spaces and punctuation that Spotify percent-encodes).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && s.is_char_boundary(i + 1)
            && s.is_char_boundary(i + 3)
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn exchange_code(
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
    let status = resp.status();
    if !status.is_success() {
        // Spotify replies with a JSON error body (invalid_client,
        // invalid_grant, invalid redirect_uri...) that makes the failure
        // actionable; surface it instead of an opaque status code.
        let body = resp.text().await.unwrap_or_default().trim().to_string();
        let detail = if body.is_empty() {
            String::new()
        } else {
            format!(": {body}")
        };
        return Err(format!("token exchange failed: HTTP {status}{detail}"));
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
    fn redirect_gtm_port() {
        let flow = OauthFlow::new("test-client-id", DEFAULT_OAUTH_PORT);
        assert_eq!(flow.redirect_uri, DEFAULT_REDIRECT_URI);
    }

    #[test]
    fn url_has_pkce() {
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
            code_from_redirect("/login?code=abc123&state=xyz", "xyz").unwrap(),
            Some("abc123".to_string())
        );
        assert_eq!(code_from_redirect("/login?state=xyz", "xyz").unwrap(), None);
        assert_eq!(code_from_redirect("/login", "xyz").unwrap(), None);
        // Mismatched state (CSRF) must be rejected even when a code is present.
        assert_eq!(
            code_from_redirect("/login?code=abc123&state=evil", "xyz").unwrap(),
            None
        );
        // Missing state with a code is also rejected.
        assert_eq!(
            code_from_redirect("/login?code=abc123", "xyz").unwrap(),
            None
        );
    }

    #[test]
    fn param_extraction_generic() {
        // Last.fm-style redirect carrying a `token` and no `state`.
        assert_eq!(
            param_from_redirect("/login?token=abc123", "token", None).unwrap(),
            Some("abc123".to_string())
        );
        // Unrelated requests (favicons, prefetches) yield None and the
        // acceptor keeps waiting for a genuine redirect.
        assert_eq!(
            param_from_redirect("/favicon.ico", "token", None).unwrap(),
            None
        );
        // With an expected state, the param must arrive with a matching state.
        assert_eq!(
            param_from_redirect("/login?token=t&state=xyz", "token", Some("xyz")).unwrap(),
            Some("t".to_string())
        );
        assert_eq!(
            param_from_redirect("/login?token=t&state=evil", "token", Some("xyz")).unwrap(),
            None
        );
    }

    #[test]
    fn redirect_error_surfaced() {
        // Spotify rejects the authorize request (e.g. an unknown scope) and
        // redirects back with an `error` param; the flow must fail fast with
        // the human-readable message instead of waiting out the timeout.
        assert_eq!(
            code_from_redirect("/login?error=invalid_scope&state=xyz", "xyz"),
            Err("invalid_scope".to_string())
        );
        assert_eq!(
            code_from_redirect(
                "/login?error=access_denied&error_description=The%20user%20denied%20access&state=xyz",
                "xyz"
            ),
            Err("access_denied (The user denied access)".to_string())
        );
    }

    #[test]
    fn urlencoding() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("plain"), "plain");
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(
            percent_decode("The%20user%20denied%20access"),
            "The user denied access"
        );
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%2Gz"), "bad%2Gz");
    }
}
