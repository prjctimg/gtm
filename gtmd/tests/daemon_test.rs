use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use gtm_core::ipc::{DaemonReq, DaemonRes, QueueAction};

use gtmd::config::{DaemonArgs, DaemonConfig};
use gtmd::daemon::Daemon;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn test_paths() -> (PathBuf, PathBuf) {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut socket = std::env::temp_dir();
    socket.push(format!("gtmd_test_{}_{}.sock", std::process::id(), n));
    let mut db = std::env::temp_dir();
    db.push(format!("gtmd_test_{}_{}.db", std::process::id(), n));
    (socket, db)
}

fn test_config() -> DaemonConfig {
    let (socket, db) = test_paths();
    let args = DaemonArgs {
        socket: Some(socket.to_string_lossy().to_string()),
        library: Some(db.to_string_lossy().to_string()),
        config: None,
        verbose: false,
        test_mode: true,
        backend: None,
    };
    let config = DaemonConfig::load(&args);
    let _ = std::fs::remove_file(&config.socket_path);
    let _ = std::fs::remove_file(&config.library_path);
    config
}

fn cleanup(config: &DaemonConfig) {
    let _ = std::fs::remove_file(&config.socket_path);
    let _ = std::fs::remove_file(&config.library_path);
}

/// A buffered reader that can handle both JSON response lines and binary
/// WireFrame event frames on the same stream, using the first-byte heuristic
/// (same logic as `IpcWorker::parse` in gtm-core/src/client.rs).
struct TestReader {
    stream: tokio::net::unix::OwnedReadHalf,
    buf: Vec<u8>,
}

impl TestReader {
    fn new(stream: tokio::net::unix::OwnedReadHalf) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    /// Read the next JSON response, silently discarding any binary event frames
    /// that arrive before it.
    async fn read_response(&mut self) -> DaemonRes {
        loop {
            // Try to parse from buffered data first.
            if let Some(res) = self.try_parse_response() {
                return res;
            }
            // Need more data from the socket.
            let mut tmp = [0u8; 8192];
            let n = self.stream.read(&mut tmp).await.expect("read from socket");
            if n == 0 {
                panic!(
                    "connection closed before response (buf contains {} bytes: {:?})",
                    self.buf.len(),
                    &self.buf[..self.buf.len().min(32)]
                );
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }

    fn try_parse_response(&mut self) -> Option<DaemonRes> {
        loop {
            if self.buf.is_empty() {
                return None;
            }
            if self.buf[0] == b'{' || self.buf[0] == b'"' {
                // JSON response line — find the newline.
                let pos = match self.buf.iter().position(|&b| b == b'\n') {
                    Some(p) => p,
                    None => return None, // incomplete line, need more data
                };
                let line = self.buf[..pos].to_vec();
                self.buf.drain(..=pos);
                let res: DaemonRes = match serde_json::from_slice(&line) {
                    Ok(r) => r,
                    Err(_) => {
                        continue; // skip bad data
                    }
                };
                return Some(res);
            }
            // Binary WireFrame: 4-byte big-endian length prefix + payload.
            if self.buf.len() < 4 {
                // Not enough for the length prefix yet — read more.
                return None;
            }
            let len =
                u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            if self.buf.len() < 4 + len {
                // Not enough for the full frame — read more.
                return None;
            }
            // Discard the binary event frame and loop to try parsing again.
            self.buf.drain(..4 + len);
        }
    }
}

async fn send_req(
    reader: &mut TestReader,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    req: &DaemonReq,
) -> DaemonRes {
    let line = serde_json::to_string(req).unwrap() + "\n";
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
    let res = reader.read_response().await;
    res
}

async fn daemon_handle() -> (tokio::task::JoinHandle<()>, DaemonConfig) {
    let config = test_config();
    let mut daemon = Daemon::new(config.clone()).unwrap();
    let handle = tokio::spawn(async move {
        daemon.run().await.ok();
    });

    // Wait for daemon event loop to start
    tokio::time::sleep(Duration::from_secs(2)).await;
    (handle, config)
}

async fn connect(socket_path: &PathBuf) -> (TestReader, tokio::net::unix::OwnedWriteHalf) {
    let stream = UnixStream::connect(socket_path).await.unwrap();
    let (reader_half, writer_half) = stream.into_split();
    (TestReader::new(reader_half), writer_half)
}

#[tokio::test]
async fn test_ping_pong() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    let res = send_req(&mut reader, &mut writer, &DaemonReq::Ping).await;
    assert!(matches!(res, DaemonRes::Pong));

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_get_status() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    let res = send_req(&mut reader, &mut writer, &DaemonReq::GetStatus).await;
    assert!(matches!(res, DaemonRes::Status { .. }));

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_queue_list() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::QueueState { .. }));

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_queue_add_and_list() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Add {
                path: "/tmp/test.opus".into(),
                position: None,
            },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok { .. }));

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        } => {
            assert_eq!(tracks.len(), 1, "expected 1 track in queue");
            assert_eq!(tracks[0].path, "/tmp/test.opus");
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected QueueState, got {res:?}"),
    }

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_queue_add_multiple() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    for path in &["/tmp/a.opus", "/tmp/b.opus", "/tmp/c.opus"] {
        let res = send_req(
            &mut reader,
            &mut writer,
            &DaemonReq::Queue {
                action: QueueAction::Add {
                    path: path.to_string(),
                    position: None,
                },
            },
        )
        .await;
        assert!(matches!(res, DaemonRes::Ok { .. }));
    }

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        } => {
            assert_eq!(tracks.len(), 3);
            assert_eq!(tracks[0].path, "/tmp/a.opus");
            assert_eq!(tracks[1].path, "/tmp/b.opus");
            assert_eq!(tracks[2].path, "/tmp/c.opus");
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected QueueState, got {res:?}"),
    }

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_queue_remove() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    for path in &["/tmp/x.opus", "/tmp/y.opus"] {
        send_req(
            &mut reader,
            &mut writer,
            &DaemonReq::Queue {
                action: QueueAction::Add {
                    path: path.to_string(),
                    position: None,
                },
            },
        )
        .await;
    }

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Remove { index: 0 },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok { .. }));

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState {
            queue: tracks,
            cursor: _,
            ..
        } => {
            assert_eq!(tracks.len(), 1);
            assert_eq!(tracks[0].path, "/tmp/y.opus");
        }
        _ => panic!("expected QueueState, got {res:?}"),
    }

    handle.abort();
    cleanup(&config);
}

#[tokio::test]
async fn test_queue_clear() {
    let (handle, config) = daemon_handle().await;
    let (mut reader, mut writer) = connect(&config.socket_path).await;

    send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Add {
                path: "/tmp/z.opus".into(),
                position: None,
            },
        },
    )
    .await;

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Clear,
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok { .. }));

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        } => {
            assert_eq!(tracks.len(), 0);
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected QueueState, got {res:?}"),
    }

    handle.abort();
    cleanup(&config);
}
