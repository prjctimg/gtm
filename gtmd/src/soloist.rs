// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Spotify Soloist playback bridge: spawns the official `soloist` daemon and
// drives it over its local WebSocket API so gtm can control Spotify playback.
//
// This is free software released under the GPL-3.0 license.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use gtm_core::spotify::SoloistStatus;

const KEY_FILE: &str = "soloist.key";
const KEY_PERMS: u32 = 0o600;
const DEVICE_NAME: &str = "gtm";
/// Wait up to this long for the `soloist` daemon to write `ws.port`.
const WS_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Messages produced by the [`SoloistManager`] and consumed by the daemon's
/// event loop, which owns the state lock and event broadcast.
#[derive(Debug)]
pub enum SoloistMsg {
    /// WebSocket connection is live and `auth_state` has been received.
    Connected {
        logged_in: bool,
        device_name: Option<String>,
    },
    /// The WebSocket connection dropped (or the process exited).
    Disconnected(String),
    /// A parsed Soloist event (`type` field + full payload).
    Event { name: String, data: Value },
    /// Startup failure: the process could not be spawned, the endpoint was
    /// never written, or the WebSocket connection was refused.
    Failed(String),
}

/// Manages the lifecycle of the `soloist` daemon process and its WebSocket
/// connection. The daemon process runs with the user's own API key and the
/// key file is stored with 0600 permissions. The `soloist` binary itself is
/// not bundled: users install it from the official Spotify downloads page.
pub struct SoloistManager {
    config_dir: PathBuf,
    key: Option<String>,
    child: Option<Child>,
    ws_tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
}

impl SoloistManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            key: None,
            child: None,
            ws_tx: Mutex::new(None),
        }
    }

    pub fn key_path(&self) -> PathBuf {
        self.config_dir.join(KEY_FILE)
    }

    /// True when a key file exists on disk (regardless of load status).
    pub fn has_key(&self) -> bool {
        self.key_path().exists()
    }

    /// Load the persisted API key into memory, if present.
    pub fn load_key(&mut self) {
        if let Ok(raw) = std::fs::read_to_string(self.key_path()) {
            let key = raw.trim().to_string();
            if !key.is_empty() {
                self.key = Some(key);
            }
        }
    }

    /// Persist the API key with 0600 permissions.
    pub fn save_key(&mut self, key: &str) -> Result<(), String> {
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err("empty Soloist API key".into());
        }
        std::fs::create_dir_all(&self.config_dir).map_err(|e| format!("create config dir: {e}"))?;
        let path = self.key_path();
        std::fs::write(&path, &key).map_err(|e| format!("write key file: {e}"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(KEY_PERMS))
            .map_err(|e| format!("set key permissions: {e}"))?;
        self.key = Some(key);
        Ok(())
    }

    /// Delete the persisted key and drop the in-memory copy.
    pub fn clear_key(&mut self) {
        self.key = None;
        match std::fs::remove_file(self.key_path()) {
            Ok(()) => info!("removed soloist key file"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("failed to remove soloist key file: {e}"),
        }
    }

    /// Spawn the `soloist` daemon and connect to its WebSocket endpoint.
    /// On success the reader task streams [`SoloistMsg`]s into `msg_tx` and a
    /// writer task forwards commands pushed through [`SoloistManager::send`].
    pub async fn start(&mut self, msg_tx: mpsc::UnboundedSender<SoloistMsg>) {
        if self.child.is_some() {
            warn!("soloist already running; ignoring start request");
            return;
        }
        let key = match &self.key {
            Some(k) => k.clone(),
            None => {
                self.load_key();
                match self.key.clone() {
                    Some(k) => k,
                    None => {
                        let _ =
                            msg_tx.send(SoloistMsg::Failed("no soloist API key configured".into()));
                        return;
                    }
                }
            }
        };

        let data_dir = self.config_dir.join("soloist-data");
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            let _ = msg_tx.send(SoloistMsg::Failed(format!("create soloist data dir: {e}")));
            return;
        }

        // The binary is user-installed (official downloads page); it is not
        // redistributed. Surface a clear error when it is missing.
        let child = match Command::new("soloist")
            .arg("-n")
            .arg(DEVICE_NAME)
            .arg("-k")
            .arg(&key)
            .arg("--ws")
            .arg("127.0.0.1:0")
            .arg("-D")
            .arg(&data_dir)
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = msg_tx.send(SoloistMsg::Failed(format!(
                    "failed to start `soloist`: {e}. Install it from the Spotify \
                     Soloist downloads page and ensure it is on PATH."
                )));
                return;
            }
        };
        self.child = Some(child);
        self.ws_tx.lock().await.take();

        // Poll for the ws.port/ws.addr runtime files the daemon writes on
        // startup (port 0 means the OS picked a free port).
        let deadline = tokio::time::Instant::now() + WS_READY_TIMEOUT;
        let (addr, port) = loop {
            match read_ws_endpoint(&data_dir) {
                Some(ep) => break ep,
                None => {
                    // Early exit (e.g. expired build, exit code 10)?
                    if let Some(status) = self
                        .child
                        .as_mut()
                        .and_then(|c| c.try_wait().ok())
                        .flatten()
                    {
                        let _ = msg_tx.send(SoloistMsg::Failed(format!(
                            "soloist exited during startup with status {status}; \
                             rebuilds expire (exit code 10) - install a newer build"
                        )));
                        self.child = None;
                        return;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        let _ = msg_tx.send(SoloistMsg::Failed(
                            "timed out waiting for soloist WebSocket endpoint".into(),
                        ));
                        self.stop().await;
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };

        let ws_url = format!("ws://{addr}:{port}");
        debug!("connecting to soloist WebSocket at {ws_url}");
        let (ws, _resp) = match tokio_tungstenite::connect_async(&ws_url).await {
            Ok(c) => c,
            Err(e) => {
                let _ = msg_tx.send(SoloistMsg::Failed(format!(
                    "soloist WebSocket connect failed: {e}"
                )));
                self.stop().await;
                return;
            }
        };

        // Split into a reader (streams messages) and writer (accepts commands).
        let (write, read) = ws.split();
        let (ws_tx, ws_rx) = mpsc::unbounded_channel();
        *self.ws_tx.lock().await = Some(ws_tx);

        let msg_tx_reader = msg_tx.clone();
        tokio::spawn(async move {
            Self::reader_loop(read, msg_tx_reader).await;
        });
        let msg_tx_writer = msg_tx.clone();
        tokio::spawn(async move {
            Self::writer_loop(write, ws_rx).await;
            let _ = msg_tx_writer.send(SoloistMsg::Disconnected("soloist WebSocket closed".into()));
        });

        info!("soloist playback bridge started at {ws_url}");
    }

    /// Terminate the daemon process and reset the bridge state.
    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.ws_tx.lock().await.take();
    }

    /// Queue a JSON command for the writer task. Returns true if accepted.
    pub async fn send(&self, json: String) -> bool {
        let tx = self.ws_tx.lock().await;
        match tx.as_ref() {
            Some(tx) => tx.send(json).is_ok(),
            None => false,
        }
    }

    /// Serialize a control command in the documented wire format.
    pub fn cmd(command: &str, fields: Vec<(&str, Value)>) -> String {
        let mut obj = serde_json::Map::new();
        obj.insert("type".into(), Value::String("command".into()));
        obj.insert("command".into(), Value::String(command.into()));
        for (k, v) in fields {
            obj.insert(k.into(), v);
        }
        serde_json::to_string(&Value::Object(obj)).unwrap_or_default()
    }

    /// Parse an incoming Soloist event into a [`SoloistMsg`].
    fn parse_message(text: &str) -> Option<SoloistMsg> {
        let parsed: Value = serde_json::from_str(text).ok()?;
        let name = parsed.get("type")?.as_str()?.to_string();
        Some(SoloistMsg::Event { name, data: parsed })
    }

    async fn reader_loop<R>(mut read: R, msg_tx: mpsc::UnboundedSender<SoloistMsg>)
    where
        R: futures::StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
    {
        while let Some(item) = read.next().await {
            match item {
                Ok(Message::Text(text)) => {
                    if let Some(msg) = Self::parse_message(&text) {
                        if msg_tx.send(msg).is_err() {
                            return;
                        }
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => {
                    let _ = msg_tx.send(SoloistMsg::Disconnected(
                        "soloist closed the connection".into(),
                    ));
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = msg_tx.send(SoloistMsg::Disconnected(e.to_string()));
                    return;
                }
            }
        }
        let _ = msg_tx.send(SoloistMsg::Disconnected("soloist connection closed".into()));
    }

    async fn writer_loop<S>(mut write: S, mut ws_rx: mpsc::UnboundedReceiver<String>)
    where
        S: futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        while let Some(json) = ws_rx.recv().await {
            if let Err(e) = write.send(Message::Text(json.into())).await {
                warn!("soloist command send failed: {e}");
                return;
            }
        }
    }
}

fn read_ws_endpoint(data_dir: &std::path::Path) -> Option<(String, u16)> {
    let addr = std::fs::read_to_string(data_dir.join("ws.addr")).ok()?;
    let port = std::fs::read_to_string(data_dir.join("ws.port")).ok()?;
    let addr = addr.trim();
    let port: u16 = port.trim().parse().ok()?;
    if addr.is_empty() {
        return None;
    }
    Some((addr.to_string(), port))
}

/// Build a `SoloistStatus` reflecting the currently-known bridge state.
/// The daemon is the source of truth for live status; this covers the
/// persisted key + process liveness only.
pub fn static_status(config_dir: &std::path::Path, running: bool) -> SoloistStatus {
    SoloistStatus {
        key_set: config_dir.join(KEY_FILE).exists(),
        running,
        connected: false,
        logged_in: false,
        user: None,
        device: None,
        track: None,
        playing: false,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_wire_format() {
        let json = SoloistManager::cmd("seek", vec![("position_ms", Value::from(30000))]);
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "command");
        assert_eq!(parsed["command"], "seek");
        assert_eq!(parsed["position_ms"], 30000);
    }

    #[test]
    fn cmd_no_fields() {
        let json = SoloistManager::cmd("pause", vec![]);
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "command");
        assert_eq!(parsed["command"], "pause");
    }

    #[test]
    fn parse_message_event() {
        let msg = SoloistManager::parse_message(r#"{"type":"playback_changed","status":"paused"}"#)
            .expect("event should parse");
        match msg {
            SoloistMsg::Event { name, data } => {
                assert_eq!(name, "playback_changed");
                assert_eq!(data["status"], "paused");
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }
}
