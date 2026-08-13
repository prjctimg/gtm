use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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

/// Drain any pending binary WireFrame events from the reader.
/// Returns without blocking if no data is available.
async fn drain_events(reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>) {
    let mut buf = [0u8; 65536];
    // Drain any data already buffered in the BufReader
    let n = reader.read(&mut buf).await.unwrap_or(0);
    if n == 0 {
        return;
    }
    // One more attempt with a short timeout for late-arriving frames
    let r = tokio::time::timeout(Duration::from_millis(10), reader.read(&mut buf)).await;
    let _ = r;
}

/// Send a JSON request and read the JSON response line.
async fn send_req(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    req: &DaemonReq,
) -> DaemonRes {
    let line = serde_json::to_string(req).unwrap() + "\n";
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).await.unwrap();
    serde_json::from_str(resp_line.trim()).unwrap()
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

async fn connect(
    socket_path: &PathBuf,
) -> (
    BufReader<tokio::net::unix::OwnedReadHalf>,
    tokio::net::unix::OwnedWriteHalf,
) {
    let stream = UnixStream::connect(socket_path).await.unwrap();
    let (reader_half, writer_half) = stream.into_split();
    let reader = BufReader::new(reader_half);
    (reader, writer_half)
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

    // Add a track — this emits a QueueChanged binary event frame
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

    // Drain any pending binary event frames before the next command
    drain_events(&mut reader).await;

    // List and verify
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState { tracks, cursor, .. } => {
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

    // Add multiple tracks
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
        drain_events(&mut reader).await;
    }

    // Verify all three are in the queue
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState { tracks, cursor, .. } => {
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

    // Add two tracks
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
        drain_events(&mut reader).await;
    }

    // Remove the first track
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Remove { index: 0 },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok { .. }));
    drain_events(&mut reader).await;

    // Verify only the second track remains
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
            tracks, cursor: _, ..
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

    // Add a track
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
    drain_events(&mut reader).await;

    // Clear the queue
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Clear,
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok { .. }));
    drain_events(&mut reader).await;

    // Verify empty
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState { tracks, cursor, .. } => {
            assert_eq!(tracks.len(), 0);
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected QueueState, got {res:?}"),
    }

    handle.abort();
    cleanup(&config);
}
