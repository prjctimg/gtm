// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon lifecycle helpers: locate the gtmd binary and ensure a live daemon
// is running at a given socket before a client connects.
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::shared::ipc::{DaemonReq, WireReq};
use crate::shared::paths::resolve_pid_file;

/// PID recorded by a running (or crashed) `gtmd`, if the pidfile exists and
/// parses.
pub fn read_daemon_pid() -> Option<u32> {
    let raw = std::fs::read_to_string(resolve_pid_file()).ok()?;
    raw.trim().parse::<u32>().ok()
}

/// Whether a PID is currently alive, via `kill(pid, 0)` (returns 0 for a
/// running process, `ESRCH` for a dead one).
pub fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 only probes for existence and never
    // delivers a signal.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Terminate a daemon process, escalating from SIGTERM to SIGKILL.
pub fn terminate_daemon(pid: u32) {
    // SAFETY: well-formed pid/signal passed to the OS kill syscall.
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
}

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
    if stream
        .write_all(format!("{ping}\n").as_bytes())
        .await
        .is_err()
    {
        return false;
    }
    matches!(
        tokio::time::timeout(timeout, stream.read(&mut buf)).await,
        Ok(Ok(n)) if n > 0
    )
}

/// Ensure a live daemon answers at `socket_path`, spawning `gtmd` detached
/// when the socket is absent or stale. Waits up to ~12 s for startup. If a
/// previous daemon process is recorded in the pidfile but does not answer,
/// it is terminated so a fresh instance can bind (never two daemons).
pub async fn ensure_daemon_running(socket_path: &Path) -> Result<(), String> {
    // Fast path: an existing socket that answers ping means a live daemon.
    if socket_path.exists() && ping_socket(socket_path, std::time::Duration::from_millis(100)).await
    {
        return Ok(());
    }

    // The socket is missing or stale. A pidfile with a live process is a
    // leftover/hung daemon (mid-crash, unresponsive, or a partially shut-down
    // instance); restart it instead of leaving a duplicate. Give it a second
    // ping in case it is still mid-startup.
    if let Some(pid) = read_daemon_pid()
        && pid_is_alive(pid)
    {
        if socket_path.exists()
            && ping_socket(socket_path, std::time::Duration::from_millis(500)).await
        {
            return Ok(());
        }
        terminate_daemon(pid);
        for _ in 0..30 {
            if !pid_is_alive(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if pid_is_alive(pid) {
            // SAFETY: well-formed pid/signal passed to the OS kill syscall.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    }

    // Stale socket: the previous daemon died without cleaning up.
    let _ = std::fs::remove_file(socket_path);
    let pulse_path = socket_path.with_extension("pulse");
    let _ = std::fs::remove_file(&pulse_path);

    spawn_daemon(socket_path).await
}

/// Probe whether the running daemon understands the `tidal_status` IPC that
/// the Tidal setup flow depends on. Daemons predating that command answer
/// `unknown command: tidal_status`; any other reply (a status object or an
/// unrelated error) means the command exists. A busy or unresponsive daemon
/// is treated as compatible (the regular IPC layer surfaces real errors), so
/// a healthy daemon is never restarted on a probe timeout.
async fn probe_tidal_status(socket_path: &Path) -> bool {
    let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await else {
        return true;
    };
    let Ok(req) = serde_json::to_string(&WireReq {
        id: 1,
        cmd: "tidal_status".to_string(),
        params: serde_json::to_value(DaemonReq::TidalStatus).unwrap_or_default(),
    }) else {
        return true;
    };
    if stream
        .write_all(format!("{req}\n").as_bytes())
        .await
        .is_err()
    {
        return true;
    }
    let mut buf = [0u8; 1024];
    match tokio::time::timeout(std::time::Duration::from_millis(800), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            !text.contains("unknown command")
        }
        _ => true,
    }
}

/// Ensure the running daemon matches this client's IPC feature set. An old
/// daemon can't answer the Tidal link-status command (`tidal_status`); when
/// the probe catches one, the daemon is restarted from this client's own
/// `gtmd` binary so the protocol always matches. Call after
/// [`ensure_daemon_running`].
pub async fn ensure_daemon_version(socket_path: &Path) -> Result<(), String> {
    if probe_tidal_status(socket_path).await {
        return Ok(());
    }

    // Outdated daemon: stop it (escalating to SIGKILL) and spawn a fresh one.
    if let Some(pid) = read_daemon_pid()
        && pid_is_alive(pid)
    {
        terminate_daemon(pid);
        for _ in 0..30 {
            if !pid_is_alive(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if pid_is_alive(pid) {
            // SAFETY: well-formed pid/signal passed to the OS kill syscall.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    }
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(socket_path.with_extension("pulse"));

    spawn_daemon(socket_path).await?;
    if probe_tidal_status(socket_path).await {
        Ok(())
    } else {
        Err("gtmd is too old to serve Tidal setup; restart it and try again".into())
    }
}

/// Spawn a fresh `gtmd` at `socket_path` and wait for it to answer a ping.
async fn spawn_daemon(socket_path: &Path) -> Result<(), String> {
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
        // A live pidfile with a dead socket means our spawn was beaten by (or
        // starved out by) an existing daemon; adopt it rather than erroring.
        if let Some(pid) = read_daemon_pid()
            && pid_is_alive(pid)
            && ping_socket(socket_path, std::time::Duration::from_millis(500)).await
        {
            return Ok(());
        }
    }

    Err(format!(
        "gtmd did not become ready at {}",
        socket_path.display()
    ))
}
