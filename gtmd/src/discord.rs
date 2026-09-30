// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// Discord Rich Presence over Discord's local IPC socket.
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, info, warn};

use crate::daemon::DaemonInner;
use gtm::shared::global::PlaybackStatus;
use gtm::shared::track::TrackInfo;

/// Discord IPC opcodes. The first message on a socket is a handshake; after
/// that the client only sends frames and ignores whatever comes back, apart
/// from pings.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_PING: u32 = 3;

/// Discord numbers its local IPC sockets 0..9 and picks the first it can bind.
const MAX_SOCKETS: u8 = 10;

/// How often the track on air is re-read. Presence has no change event, so the
/// updater polls; the comparison against the last payload is what makes this
/// cheap, and the interval also bounds how long a stopped track lingers.
const POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// The candidate IPC socket paths, most likely first.
///
/// Linux and Termux use `$XDG_RUNTIME_DIR`; Discord also falls back to `/tmp`
/// when the runtime dir is unset, and that is where it lands on a bare
/// container, so both are tried.
pub fn sockets() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        for i in 0..MAX_SOCKETS {
            out.push(PathBuf::from(&dir).join(format!("discord-ipc-{i}")));
        }
    }
    for i in 0..MAX_SOCKETS {
        out.push(PathBuf::from(format!("/tmp/discord-ipc-{i}")));
    }
    out
}

/// Wrap a payload in Discord's framing: little-endian opcode, little-endian
/// length, then the JSON.
pub fn frame(op: u32, payload: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Discord does not send large frames, and an unbounded read from a local
/// socket that something else opened is a way to allocate a lot.
fn valid_len(len: usize) -> bool {
    (1..=64 * 1024).contains(&len)
}

/// Read one framed message. Returns `None` at end of stream or on a length
/// that cannot be right, both of which mean the socket is no longer usable.
pub async fn read_frame(stream: &mut UnixStream) -> Option<(u32, serde_json::Value)> {
    let mut head = [0u8; 8];
    stream.read_exact(&mut head).await.ok()?;
    let op = u32::from_le_bytes(head[..4].try_into().ok()?);
    let len = u32::from_le_bytes(head[4..].try_into().ok()?) as usize;
    if !valid_len(len) {
        return None;
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.ok()?;
    Some((op, serde_json::from_slice(&body).ok()?))
}

/// The activity fields, kept separate from the wire shape so they can be
/// asserted without constructing a socket.
#[derive(Serialize, PartialEq, Debug)]
pub struct Activity {
    pub details: String,
    pub state: String,
    /// Seconds since the epoch when this track started playing.
    pub start: i64,
    /// Seconds since the epoch when it ends, absent for a live stream.
    pub end: Option<i64>,
}

impl Activity {
    /// Build from the track on air.
    ///
    /// `now` is passed in rather than read from the clock so the timestamps are
    /// testable.
    pub fn of(track: &TrackInfo, now: i64) -> Self {
        let artist = if track.artist.trim().is_empty() {
            "Unknown artist"
        } else {
            track.artist.as_str()
        };
        let album = track.album.trim();
        let state = if album.is_empty() {
            artist.to_string()
        } else {
            format!("{artist} · {album}")
        };
        Self {
            details: track.title.clone(),
            state,
            start: now,
            // No end for a live stream: a timestamp that never arrives reads as
            // a track still loading.
            end: (track.duration > 0.0).then(|| now + track.duration as i64),
        }
    }
}

/// The `SET_ACTIVITY` payload.
fn set_activity(activity: Option<&Activity>) -> serde_json::Value {
    let args = serde_json::json!({
        "pid": std::process::id(),
        "activity": activity,
    });
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": args,
        "nonce": format!("{}", std::process::id()),
    })
}

/// Keep Discord's presence in step with the track on air until the process
/// exits. A failure to reach Discord is logged once and retried on the next
/// poll, so a client started later still gets a presence.
pub(crate) async fn serve(inner: std::sync::Arc<DaemonInner>, config_dir: std::path::PathBuf) {
    let mut sock: Option<UnixStream> = None;
    let mut last: Option<Activity> = None;
    // Re-read every poll rather than capturing the id once. The `Alt+X` form
    // writes config.toml and the user should not have to restart the daemon
    // for it to take effect; the file read is a few hundred bytes.
    let mut app_id: Option<String> = None;

    loop {
        tokio::time::sleep(POLL).await;

        let id = read_id(&config_dir);
        if id != app_id {
            if let Some(ref v) = id {
                info!("Discord presence enabled for app {v}");
            } else {
                info!("Discord presence disabled");
            }
            app_id = id;
            sock = None;
            last = None;
        }
        let Some(app_id) = app_id.clone() else {
            continue;
        };

        let activity = {
            let state = inner.state.read().await;
            let playing = state.status == PlaybackStatus::Playing;
            // Track start, not now: the daemon's position is authoritative,
            // and re-anchoring to the poll would make the elapsed time in
            // Discord jump backwards on every tick.
            state
                .current_track
                .as_ref()
                .filter(|_| playing)
                .map(|t| Activity::of(t, now_secs() - state.time_pos as i64))
        };

        if sock.is_none() {
            match connect(&app_id).await {
                Ok(s) => {
                    sock = Some(s);
                    last = None;
                }
                Err(e) => {
                    debug!("Discord IPC unavailable: {e}");
                    continue;
                }
            }
        }
        let Some(s) = sock.as_mut() else { continue };

        // Only send on a change, plus a repeat while idle so a presence
        // cleared on the Discord side comes back.
        let unchanged = match (&last, &activity) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        };
        if unchanged && activity.is_some() {
            continue;
        }

        if let Err(e) = send(s, activity.as_ref()).await {
            debug!("Discord presence write failed: {e}");
            sock = None;
            last = None;
            continue;
        }
        last = activity;
    }
}

/// Read `discord_app_id` from config.toml.
fn read_id(config_dir: &std::path::Path) -> Option<String> {
    let toml = std::fs::read_to_string(config_dir.join("config.toml")).ok()?;
    let v: toml::Value = toml::from_str(&toml).ok()?;
    let raw = match v.get("discord_app_id") {
        Some(toml::Value::String(s)) => s.trim().to_string(),
        Some(toml::Value::Integer(i)) => i.to_string(),
        _ => return None,
    };
    (!raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit())).then_some(raw)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn connect(app_id: &str) -> std::io::Result<UnixStream> {
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no discord ipc socket");
    for path in sockets() {
        let Ok(mut s) = UnixStream::connect(&path).await else {
            continue;
        };
        let hs = frame(
            OP_HANDSHAKE,
            &serde_json::json!({ "v": 1, "client_id": app_id }),
        );
        s.write_all(&hs).await?;
        // The handshake reply is a frame carrying the user's name and avatar.
        // Reading it confirms the socket is really Discord's, which a stale
        // file left by a crashed client would otherwise fake.
        match read_frame(&mut s).await {
            Some((OP_FRAME | OP_PING, _)) => return Ok(s),
            _ => {
                last = std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is not a discord ipc socket", path.display()),
                )
            }
        }
    }
    Err(last)
}

async fn send(s: &mut UnixStream, activity: Option<&Activity>) -> std::io::Result<()> {
    s.write_all(&frame(OP_FRAME, &set_activity(activity)))
        .await?;
    s.flush().await
}

/// Clear the presence at shutdown, so Discord does not show a track that
/// stopped when the daemon exited.
pub async fn clear(app_id: &str) {
    let Ok(mut s) = connect(app_id).await else {
        return;
    };
    if let Err(e) = send(&mut s, None).await {
        warn!("could not clear Discord presence: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_little_endian_op_then_length() {
        let f = frame(1, &serde_json::json!({ "a": 1 }));
        assert_eq!(u32::from_le_bytes(f[0..4].try_into().unwrap()), OP_FRAME);
        let len = u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize;
        assert_eq!(len, f.len() - 8);
        let v: serde_json::Value = serde_json::from_slice(&f[8..]).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn a_live_stream_has_no_end_timestamp() {
        let t = TrackInfo {
            title: "Station".into(),
            artist: "SomaFM".into(),
            duration: 0.0,
            ..Default::default()
        };
        let a = Activity::of(&t, 1_000);
        assert_eq!(a.end, None);
        assert_eq!(a.start, 1_000);
    }

    #[test]
    fn a_finite_track_gets_an_end_timestamp() {
        let t = TrackInfo {
            title: "T".into(),
            artist: "A".into(),
            duration: 300.0,
            ..Default::default()
        };
        let a = Activity::of(&t, 1_000);
        assert_eq!(a.end, Some(1_300));
    }

    /// An empty artist or album must not produce "A · " or a leading dot.
    #[test]
    fn missing_metadata_degrades_cleanly() {
        let t = TrackInfo {
            title: "T".into(),
            artist: String::new(),
            album: String::new(),
            duration: 10.0,
            ..Default::default()
        };
        let a = Activity::of(&t, 0);
        assert_eq!(a.state, "Unknown artist");
        assert!(!a.state.contains('·'));

        let t = TrackInfo {
            title: "T".into(),
            artist: "A".into(),
            album: "  ".into(),
            duration: 10.0,
            ..Default::default()
        };
        assert_eq!(Activity::of(&t, 0).state, "A");
    }

    /// A cleared presence must send a null activity, not an empty object:
    /// Discord keeps showing the previous one otherwise.
    #[test]
    fn clear_sends_a_null_activity() {
        let v = set_activity(None);
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert!(v["args"]["activity"].is_null(), "{v}");
        let v = set_activity(Some(&Activity::of(&TrackInfo::default(), 0)));
        assert!(!v["args"]["activity"].is_null());
    }

    #[test]
    fn the_sweep_includes_both_roots() {
        let s = sockets();
        assert!(
            s.iter().any(|p| p.starts_with("/tmp/discord-ipc-")),
            "{s:?}"
        );
        assert!(s.len() as u8 >= MAX_SOCKETS);
    }

    /// A hostile or stale socket must not be able to make the daemon allocate
    /// an arbitrary buffer. This asserts the guard the reader actually uses.
    #[test]
    fn absurd_frame_lengths_are_refused() {
        assert!(!valid_len(0));
        assert!(!valid_len(64 * 1024 + 1));
        assert!(!valid_len(usize::MAX));
        assert!(valid_len(1));
        assert!(valid_len(64 * 1024));
    }
}
