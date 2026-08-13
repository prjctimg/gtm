// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// YouTube search via yt-dlp with rate limiting and timeout
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, warn};

use gtm_core::state::YTFilter;
use gtm_core::track::{StreamInfo, YTSearchResult};

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT: usize = 2;
const SEARCH_COUNT: usize = 10;

/// Owns the interactive yt-dlp search pipeline. Each new search cancels any
/// in-flight one and bumps a generation counter, so results published by a
/// superseded search are discarded by [`YoutubeManager::poll_results`].
pub struct YoutubeManager {
    cancel: Option<oneshot::Sender<()>>,
    active_task: Option<JoinHandle<()>>,
    semaphore: Arc<Semaphore>,
    cookie_file: Option<PathBuf>,
    generation: Arc<AtomicU64>,
    current_gen: u64,
    last_query: String,
    results_tx: mpsc::UnboundedSender<(u64, Vec<YTSearchResult>)>,
    results_rx: mpsc::UnboundedReceiver<(u64, Vec<YTSearchResult>)>,
}

impl Default for YoutubeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl YoutubeManager {
    pub fn new() -> Self {
        let (results_tx, results_rx) = mpsc::unbounded_channel();
        Self {
            cancel: None,
            active_task: None,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            cookie_file: None,
            generation: Arc::new(AtomicU64::new(0)),
            current_gen: 0,
            last_query: String::new(),
            results_tx,
            results_rx,
        }
    }

    pub fn set_cookie_file(&mut self, path: Option<String>) {
        self.cookie_file = path.map(PathBuf::from);
    }

    fn start_impl(&mut self, query: &str, filter: Option<YTFilter>) -> Result<u64, String> {
        self.cancel_current();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.current_gen = gen;
        self.last_query = query.to_string();
        while self.results_rx.try_recv().is_ok() {}

        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.cancel = Some(cancel_tx);
        let semaphore = self.semaphore.clone();
        let res_tx = self.results_tx.clone();
        let q = query.to_string();
        let handle = tokio::spawn(async move {
            let permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let results = run_search(&q, filter, cancel_rx, permit).await;
            let _ = res_tx.send((gen, results));
        });
        self.active_task = Some(handle);
        Ok(gen)
    }

    fn cancel_current(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.active_task.take() {
            handle.abort();
        }
        while self.results_rx.try_recv().is_ok() {}
    }

    /// Synchronous search: runs to completion and publishes results for
    /// [`YoutubeManager::poll_results`]. Used by internal flows (e.g. the
    /// Spotify resolver) that need results immediately.
    pub async fn search(&mut self, query: &str, filter: Option<YTFilter>) -> Result<(), String> {
        self.start_impl(query, filter)?;
        if let Some(handle) = self.active_task.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    /// Fire-and-forget search: cancels any in-flight search and kicks a new
    /// one off without waiting. The IPC handler uses this so the client's
    /// short response timeout is never hit.
    pub fn start_search(&mut self, query: &str, filter: Option<YTFilter>) {
        let _ = self.start_impl(query, filter);
    }

    pub async fn poll_results(&mut self) -> Result<Option<(String, Vec<YTSearchResult>)>, String> {
        let mut latest: Option<(u64, Vec<YTSearchResult>)> = None;
        while let Ok(entry) = self.results_rx.try_recv() {
            latest = Some(entry);
        }
        match latest {
            Some((gen, results)) if gen == self.current_gen => {
                let query = self.last_query.clone();
                Ok(Some((query, results)))
            }
            _ => Ok(None),
        }
    }

    pub async fn cancel(&mut self) {
        self.cancel_current();
    }

    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo, String> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("semaphore: {e}"))?;

        let output = timeout(
            SEARCH_TIMEOUT,
            Command::new("yt-dlp")
                .arg("-g")
                .arg("-f")
                .arg("bestaudio[ext=m4a]/bestaudio")
                .arg(url)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| "resolve timeout".to_string())?
        .map_err(|e| format!("yt-dlp: {e}"))?;

        if !output.status.success() {
            return Err("yt-dlp resolve failed".to_string());
        }

        let direct = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if direct.is_empty() {
            return Err("empty stream URL".to_string());
        }

        Ok(StreamInfo {
            url: direct,
            title: url.to_string(),
            ext: "m4a".to_string(),
            duration: 0.0,
        })
    }
}

/// Runs one yt-dlp search to completion, feeding JSON lines through
/// [`parse_yt_json`] until the stream ends, the caller cancels via
/// `cancel_rx`, or [`SEARCH_TIMEOUT`] elapses.
async fn run_search(
    query: &str,
    _filter: Option<YTFilter>,
    mut cancel_rx: oneshot::Receiver<()>,
    _permit: OwnedSemaphorePermit,
) -> Vec<YTSearchResult> {
    let search_arg = if query.starts_with("http://") || query.starts_with("https://") {
        query.to_string()
    } else {
        format!("ytsearch{}:{} official audio", SEARCH_COUNT, query)
    };

    let mut child = match Command::new("yt-dlp")
        .arg(&search_arg)
        .arg("--dump-json")
        .arg("--flat-playlist")
        .arg("--no-warnings")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("spawn yt-dlp: {e}");
            return Vec::new();
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut results = Vec::new();

    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if l.trim().is_empty() {
                            continue;
                        }
                        match parse_yt_json(&l) {
                            Ok(r) => results.push(r),
                            Err(e) => debug!("yt-dlp parse: {e}"),
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        warn!("yt-dlp read: {e}");
                        break;
                    }
                }
            }
            _ = &mut cancel_rx => {
                let _ = child.kill().await;
                results.clear();
                break;
            }
            _ = tokio::time::sleep(SEARCH_TIMEOUT) => {
                warn!("yt-dlp search timeout");
                let _ = child.kill().await;
                break;
            }
        }
    }

    let _ = child.wait().await;
    // Sort results: prefer "official audio" and "explicit" titles
    results.sort_by(|a, b| {
        let a_prio = priority(&a.title);
        let b_prio = priority(&b.title);
        b_prio.cmp(&a_prio).then(b.views.cmp(&a.views))
    });
    results
}

/// Returns a priority score for a YouTube result title.
/// Higher = more likely to be the "official" version the user wants.
fn priority(title: &str) -> u32 {
    let lower = title.to_lowercase();
    let mut score = 0u32;
    let keywords = [
        ("official audio", 20),
        ("official music video", 18),
        ("official video", 15),
        ("explicit", 10),
        ("official", 5),
        ("audio", 3),
        ("lyric", 1),
    ];
    for (kw, pts) in &keywords {
        if lower.contains(kw) {
            score += pts;
        }
    }
    score
}

fn parse_yt_json(line: &str) -> Result<YTSearchResult, String> {
    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("json: {e}"))?;
    let id = v
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_playlist = v.get("_type").and_then(|v| v.as_str()) == Some("playlist");
    let raw_title = v
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (_artist, title) = crate::metadata_cleaner::clean_youtube_title(&raw_title);
    Ok(YTSearchResult {
        id: id.clone(),
        title,
        url: if is_playlist {
            format!("https://www.youtube.com/playlist?list={id}")
        } else {
            format!("https://www.youtube.com/watch?v={id}")
        },
        channel: v
            .get("channel")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        duration: v.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
        views: v.get("view_count").and_then(|v| v.as_u64()).unwrap_or(0),
        thumbnail: v
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        is_playlist,
    })
}
