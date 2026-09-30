// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// Read-only JSON status endpoint, so a script, a home-screen widget or a
// web page can read what is playing without attaching to the IPC socket.
//
// This is free software released under the GPL-3.0 license.

use std::net::SocketAddr;
use std::sync::Arc;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::daemon::DaemonInner;
use gtm::shared::global::{DaemonState, PlaybackStatus};

/// The payload. Deliberately the current track and nothing else.
///
/// The daemon holds a listening history, a library and a queue; the endpoint
/// exists to answer "what is playing", so it answers that. Every field here is
/// already on screen in the TUI, which makes it safe to bind to a LAN.
#[derive(Serialize)]
struct Status<'a> {
    playing: bool,
    track: Option<Track<'a>>,
}

/// Borrowed view of the current track, so the endpoint cannot drift from the
/// struct the rest of the daemon passes around.
#[derive(Serialize)]
struct Track<'a> {
    title: &'a str,
    artist: &'a str,
    album: &'a str,
    duration: f64,
    position: f64,
    /// Seconds left, or `None` for a live stream with no end.
    remaining: Option<f64>,
    cover_url: Option<&'a str>,
}

/// Serve the endpoint until the process exits.
///
/// Binding failures are logged and the daemon carries on: the endpoint is an
/// extra surface, and a port already in use must not stop playback.
pub(crate) async fn serve(inner: Arc<DaemonInner>, addr: SocketAddr) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!("status endpoint not started on {addr}: {e}");
            return;
        }
    };
    // The port is reported after the bind, so `web_addr = 0` (or any other
    // ephemeral port) can be discovered from the log.
    match listener.local_addr() {
        Ok(a) => info!("status endpoint on http://{a}/status.json"),
        Err(_) => info!("status endpoint started on {addr}"),
    }
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                debug!("status endpoint accept failed");
                continue;
            };
            let inner = inner.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(&inner, stream).await {
                    debug!("status endpoint connection from {peer} ended: {e}");
                }
            });
        }
    });
}

async fn handle(inner: &DaemonInner, mut stream: tokio::net::TcpStream) -> std::io::Result<()> {
    // Bounded: a client that opens a socket and sends nothing must not tie up
    // a task, and a client that streams a body must not be read forever.
    let mut buf = [0u8; 1024];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "request read timed out")
        })??;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");

    // `GET` and `HEAD` only. This daemon can delete files, and an endpoint on
    // the network answering any verb would be a much larger decision than the
    // one being made here.
    let method = req.split_whitespace().next().unwrap_or("");
    let (code, body) = {
        let state = inner.state.read().await;
        reply(method, path, &state)
    };

    let head = format!(
        "HTTP/1.1 {code} {}\r\n\
         content-type: application/json\r\n\
         content-length: {}\r\n\
         access-control-allow-origin: *\r\n\
         cache-control: no-store\r\n\
         connection: close\r\n\r\n",
        reason(code),
        body.len(),
    );
    stream.write_all(head.as_bytes()).await?;
    if method != "HEAD" {
        stream.write_all(&body).await?;
    }
    stream.flush().await
}

/// Map a request to a status code and body.
///
/// Split out from the socket so the routing can be exercised without binding a
/// port, which the test suite cannot do.
fn reply(method: &str, path: &str, state: &DaemonState) -> (u16, Vec<u8>) {
    match (method, path) {
        ("GET" | "HEAD", p) if p == "/status.json" || p == "/" => {
            let track = state.current_track.as_ref().map(|t| Track {
                title: &t.title,
                artist: &t.artist,
                album: &t.album,
                duration: t.duration,
                position: state.time_pos,
                // A live stream has no end, so there is no countdown to report
                // rather than one that never reaches zero.
                remaining: (t.duration > 0.0).then(|| (t.duration - state.time_pos).max(0.0)),
                cover_url: t.cover_url.as_deref().or(t.cover_path.as_deref()),
            });
            let status = Status {
                playing: state.status == PlaybackStatus::Playing,
                track,
            };
            match serde_json::to_vec(&status) {
                Ok(b) => (200, b),
                Err(e) => (500, format!("{e}").into_bytes()),
            }
        }
        ("GET" | "HEAD", _) => (404, b"not found".to_vec()),
        _ => (405, b"method not allowed".to_vec()),
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtm::shared::track::TrackInfo;

    fn playing() -> DaemonState {
        let mut s = DaemonState::new();
        s.status = PlaybackStatus::Playing;
        s.time_pos = 12.0;
        s.current_track = Some(TrackInfo {
            title: "Nachtflügel".into(),
            artist: "X".into(),
            album: "Y".into(),
            duration: 300.0,
            ..Default::default()
        });
        s
    }

    #[test]
    fn reports_the_current_track() {
        let (code, body) = reply("GET", "/status.json", &playing());
        assert_eq!(code, 200);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["playing"], true);
        assert_eq!(v["track"]["title"], "Nachtflügel");
        assert_eq!(v["track"]["remaining"], 288.0);
    }

    /// A live stream has no end, so `remaining` is absent rather than a
    /// countdown that never reaches zero.
    #[test]
    fn live_stream_has_no_remaining() {
        let mut s = playing();
        s.current_track.as_mut().unwrap().duration = 0.0;
        let (_, body) = reply("GET", "/status.json", &s);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["track"]["remaining"].is_null(), "{v}");
    }

    #[test]
    fn idle_reports_no_track() {
        let (code, body) = reply("GET", "/status.json", &DaemonState::new());
        assert_eq!(code, 200);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["playing"], false);
        assert!(v["track"].is_null(), "{v}");
    }

    /// Only reads. The daemon can delete files, and an endpoint on the
    /// network taking a write verb is a much bigger decision than the one
    /// being made here.
    #[test]
    fn writes_are_refused() {
        for m in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
            assert_eq!(reply(m, "/status.json", &playing()).0, 405, "{m}");
        }
        assert_eq!(reply("GET", "/nope", &playing()).0, 404);
        assert_eq!(reply("GET", "/", &playing()).0, 200);
    }

    /// The endpoint must not become a second IPC surface. Every field here is
    /// already on screen in the TUI; a listening history, a queue or a file
    /// path would not be.
    #[test]
    fn payload_is_the_track_and_nothing_else() {
        let (_, body) = reply("GET", "/status.json", &playing());
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let obj = v.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(keys, ["playing", "track"]);
        let t = obj["track"].as_object().unwrap();
        let mut tk: Vec<&str> = t.keys().map(String::as_str).collect();
        tk.sort();
        assert_eq!(
            tk,
            [
                "album",
                "artist",
                "cover_url",
                "duration",
                "position",
                "remaining",
                "title"
            ]
        );
    }

    #[test]
    fn reasons_cover_every_status_emitted() {
        for code in [200, 404, 405, 500] {
            assert_ne!(reason(code), "Internal Server Error", "code {code}");
        }
    }
}
