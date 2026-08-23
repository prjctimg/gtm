use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use gtm_core::ipc::{DaemonReq, DaemonRes, PROTOCOL_VERSION, QueueAction, WireReq, WireRes};

use gtmd::config::{DaemonArgs, DaemonConfig};
use gtmd::daemon::Daemon;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn test_paths() -> (PathBuf, PathBuf, PathBuf) {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut socket = std::env::temp_dir();
    socket.push(format!("gtmd_test_{}_{}.sock", std::process::id(), n));
    let mut db = std::env::temp_dir();
    db.push(format!("gtmd_test_{}_{}.db", std::process::id(), n));
    let mut config_dir = std::env::temp_dir();
    config_dir.push(format!("gtmd_test_{}_{}_config", std::process::id(), n));
    (socket, db, config_dir)
}

fn test_config() -> DaemonConfig {
    let (socket, db, config_dir) = test_paths();
    let args = DaemonArgs {
        socket: Some(socket.to_string_lossy().to_string()),
        library: Some(db.to_string_lossy().to_string()),
        // Redirect data/config/cache dirs to a per-test temp location so
        // library operations never touch the real ~/.local/share/gtm database
        // and parallel tests can't clobber each other.
        config: Some(config_dir.to_string_lossy().to_string()),
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
    let _ = std::fs::remove_dir_all(&config.data_dir);
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

    /// Read the next response envelope, silently discarding any binary event
    /// frames (and JSON event lines) that arrive before it.
    async fn read_response(&mut self) -> WireRes {
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

    fn try_parse_response(&mut self) -> Option<WireRes> {
        loop {
            if self.buf.is_empty() {
                return None;
            }
            if self.buf[0] == b'{' {
                // JSON line: either a response envelope or an event line.
                let pos = self.buf.iter().position(|&b| b == b'\n')?;
                let line = self.buf[..pos].to_vec();
                self.buf.drain(..=pos);
                match serde_json::from_slice::<WireRes>(&line) {
                    Ok(res) => return Some(res),
                    Err(_) => continue, // event line, skip it
                }
            }
            // Binary WireFrame: 4-byte big-endian length prefix + payload.
            if self.buf.len() < 4 {
                // Not enough for the length prefix yet: read more.
                return None;
            }
            let len =
                u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
            if self.buf.len() < 4 + len {
                // Not enough for the full frame: read more.
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
    let params = serde_json::to_value(req).unwrap();
    let wire = WireReq {
        id: 1,
        cmd: req.cmd_name().to_string(),
        params,
    };
    let line = serde_json::to_string(&wire).unwrap() + "\n";
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
    let res = reader.read_response().await;
    DaemonRes::from_wire(req.cmd_name(), &res)
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
    let mut reader = TestReader::new(reader_half);
    let mut writer = writer_half;

    // The daemon requires a handshake before accepting commands.
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Handshake {
            version: PROTOCOL_VERSION,
            client: "daemon_test".into(),
            client_version: None,
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Handshake { .. }));

    (reader, writer)
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
                paths: vec!["/tmp/test.opus".into()],
                position: None,
            },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok), "got {res:?}");

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
                    paths: vec![path.to_string()],
                    position: None,
                },
            },
        )
        .await;
        assert!(matches!(res, DaemonRes::Ok));
    }

    // Each Add with `position: None` queues the entry "next" (right after the
    // currently-playing head), so sequential adds land in reverse insertion
    // order after the head.
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
            assert_eq!(tracks[1].path, "/tmp/c.opus");
            assert_eq!(tracks[2].path, "/tmp/b.opus");
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
                    paths: vec![path.to_string()],
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
    assert!(matches!(res, DaemonRes::Ok), "got {res:?}");

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::List,
        },
    )
    .await;
    match res {
        DaemonRes::QueueState { queue: tracks, .. } => {
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
                paths: vec!["/tmp/z.opus".into()],
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
    assert!(matches!(res, DaemonRes::Ok));

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

/// Write a tiny PCM WAV file so the daemon can decode and "play" it.
fn create_test_wav(path: &std::path::Path, duration_secs: f64) {
    let sample_rate: u32 = 44100;
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let bytes_per_sample = bits_per_sample / 8;
    let num_samples = (sample_rate as f64 * duration_secs) as u64 * channels as u64;
    let data_size = num_samples * bytes_per_sample as u64;
    let file_size = 36 + data_size;

    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(file_size as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&(channels * bytes_per_sample).to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());
    for i in 0..num_samples {
        let sample = (i as f64 * 440.0 * 2.0 * std::f64::consts::PI / sample_rate as f64).sin();
        let val = (sample * 0.3 * i16::MAX as f64) as i16;
        wav.extend_from_slice(&val.to_le_bytes());
    }
    std::fs::write(path, &wav).unwrap();
}

/// Deleting the currently-playing track must stop playback and drop it from
/// the queue so neither the row nor the audio survives (pause-then-delete).
#[tokio::test]
async fn test_delete_playing_track() {
    let (handle, config) = daemon_handle().await;

    let audio_dir = config.data_dir.join("audio");
    std::fs::create_dir_all(&audio_dir).unwrap();
    let wav_path = audio_dir.join("delete_me.wav");
    create_test_wav(&wav_path, 30.0);

    let (mut reader, mut writer) = connect(&config.socket_path).await;

    let path = wav_path.to_str().unwrap().to_string();

    // Index the file so it gets a real library row + id.
    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Library {
            action: gtm_core::ipc::LibraryAction::Scan {
                path: audio_dir.to_string_lossy().to_string(),
            },
        },
    )
    .await;
    let DaemonRes::Tracks { tracks } = res else {
        panic!("expected Tracks, got {res:?}");
    };
    assert_eq!(tracks.len(), 1);
    let track_id = tracks[0].id;
    assert!(track_id > 0);

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Queue {
            action: QueueAction::Add {
                paths: vec![path.clone()],
                position: None,
            },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok));

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Play {
            path,
            start_pos: 0.0,
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok));

    let res = send_req(&mut reader, &mut writer, &DaemonReq::GetStatus).await;
    let DaemonRes::Status { state } = res else {
        panic!("expected Status, got {res:?}");
    };
    assert!(state.current_track.is_some(), "track should be playing");
    assert_eq!(state.queue.len(), 1);

    let res = send_req(
        &mut reader,
        &mut writer,
        &DaemonReq::Library {
            action: gtm_core::ipc::LibraryAction::RemoveTrack { id: track_id },
        },
    )
    .await;
    assert!(matches!(res, DaemonRes::Ok));

    let mut status = gtm_core::state::PlaybackStatus::Playing;
    for _ in 0..50 {
        let res = send_req(&mut reader, &mut writer, &DaemonReq::GetStatus).await;
        let DaemonRes::Status { state } = res else {
            panic!("expected Status, got {res:?}");
        };
        if state.status == gtm_core::state::PlaybackStatus::Stopped {
            status = state.status;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let res = send_req(&mut reader, &mut writer, &DaemonReq::GetStatus).await;
    let DaemonRes::Status { state } = res else {
        panic!("expected Status, got {res:?}");
    };
    assert_eq!(status, gtm_core::state::PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());
    assert!(state.queue.is_empty());

    handle.abort();
    let _ = std::fs::remove_file(&wav_path);
    cleanup(&config);
}
