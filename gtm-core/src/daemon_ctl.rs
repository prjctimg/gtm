// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon lifecycle helpers: locate the gtmd binary and ensure a live daemon
// is running at a given socket before a client connects.
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;

use crate::ipc::{DaemonReq, WireReq};

/// Locate the `gtmd` daemon binary: next to the current executable, on
/// `$PATH`, or in the canonical `/usr/bin` location.
pub fn find_gtmd_binary() -> Result<std::path::PathBuf, String> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join("gtmd");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("gtmd");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    let candidate = std::path::PathBuf::from("/usr/bin/gtmd");
    if candidate.exists() {
        return Ok(candidate);
    }

    Err("gtmd binary not found".into())
}

/// Send a `ping` wire frame and wait briefly for a reply. Returns `true` when
/// a live daemon answers.
async fn ping_socket(socket_path: &Path, timeout: std::time::Duration) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await else {
        return false;
    };
    let Ok(ping) = serde_json::to_string(&WireReq {
        id: 0,
        cmd: "ping".to_string(),
        params: serde_json::to_value(DaemonReq::Ping).unwrap_or_default(),
    }) else {
        return false;
    };
    let mut buf = [0u8; 256];
    if stream.write_all(format!("{ping}\n").as_bytes()).await.is_err() {
        return false;
    }
    matches!(
        tokio::time::timeout(timeout, stream.read(&mut buf)).await,
        Ok(Ok(n)) if n > 0
    )
}

/// Ensure a live daemon answers at `socket_path`, spawning `gtmd` detached
/// when the socket is absent or stale. Waits up to ~12 s for startup.
pub async fn ensure_daemon_running(socket_path: &Path) -> Result<(), String> {
    if socket_path.exists() {
        if ping_socket(socket_path, std::time::Duration::from_millis(100)).await {
            return Ok(());
        }
        // Stale socket: a previous daemon died without cleaning up.
        let _ = std::fs::remove_file(socket_path);
    }

    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let gtmd_path = find_gtmd_binary()?;
    let socket_arg = format!("--socket={}", socket_path.display());

    let mut child = std::process::Command::new(&gtmd_path)
        .arg(&socket_arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to start gtmd at {gtmd_path:?}: {e}"))?;

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if ping_socket(socket_path, std::time::Duration::from_millis(500)).await {
            return Ok(());
        }
    }

    Err(format!(
        "gtmd did not become ready at {}",
        socket_path.display()
    ))
}