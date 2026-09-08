// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// YouTube search and stream resolution via the InnerTube API (innertube-rs).
// No external yt-dlp (or any other subprocess) is required.
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use innertube_rs::{
    FormatFilter, FormatType, Innertube, QualityPreference, SessionOptions, StreamingFormat,
};
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::debug;

use gtm_core::global::YTFilter;
use gtm_core::track::{StreamInfo, YTSearchResult};

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT: usize = 2;

fn parse_yt_dlp_progress(line: &str) -> Option<f64> {
    if let Some(start) = line.find("[download]") {
        let rest = &line[start..];
        if let Some(pct_start) = rest.find('%') {
            let pct_str = &rest[..pct_start];
            if let Some(num_start) = pct_str.rfind(|c: char| c.is_ascii_digit() || c == '.') {
                let num_str = &pct_str[num_start..];
                return num_str.parse::<f64>().ok().map(|p| p / 100.0);
            }
        }
    }
    None
}

/// Owns the interactive InnerTube search pipeline. Each new search cancels any
/// in-flight one and bumps a generation counter, so results published by a
/// superseded search are discarded by [`YoutubeManager::poll_results`].
///
/// The underlying [`Innertube`] client is shared (it is cheaply cloneable —
/// the HTTP session and decipher engine are held behind `Arc`s), so search,
/// stream resolution and downloads all reuse one client/quickjs instance.
pub struct YoutubeManager {
    client: Option<Innertube>,
    cancel: Option<oneshot::Sender<()>>,
    active_task: Option<JoinHandle<()>>,
    semaphore: Arc<Semaphore>,
    cookie_file: Option<PathBuf>,
    generation: Arc<AtomicU64>,
    current_gen: u64,
    last_query: String,
    results_tx: mpsc::UnboundedSender<(u64, Vec<YTSearchResult>)>,
    results_rx: mpsc::UnboundedReceiver<(u64, Vec<YTSearchResult>)>,
    download_task: Option<JoinHandle<()>>,
    download_cancel: Option<oneshot::Sender<()>>,
    download_progress_tx: mpsc::UnboundedSender<DownloadProgress>,
    download_progress_rx: mpsc::UnboundedReceiver<DownloadProgress>,
    download_dir: PathBuf,
    max_concurrent_downloads: usize,
    /// Mtime of the cookie file the current InnerTube client was built with,
    /// so freshly-exported cookies (the usual fix for HTTP 403) are picked up
    /// without requiring a restart.
    client_cookie_mtime: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone)]
struct Cookie {
    domain: String,
    name: String,
    value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DownloadProgress {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub progress: f64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

impl Default for YoutubeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl YoutubeManager {
    pub fn new() -> Self {
        let (results_tx, results_rx) = mpsc::unbounded_channel();
        let (download_progress_tx, download_progress_rx) = mpsc::unbounded_channel();
        let download_dir = dirs::audio_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
            .join("gtm")
            .join("downloads");
        let _ = std::fs::create_dir_all(&download_dir);
        Self {
            client: None,
            cancel: None,
            active_task: None,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            cookie_file: None,
            generation: Arc::new(AtomicU64::new(0)),
            current_gen: 0,
            last_query: String::new(),
            results_tx,
            results_rx,
            download_task: None,
            download_cancel: None,
            download_progress_tx,
            download_progress_rx,
            download_dir,
            max_concurrent_downloads: 2,
            client_cookie_mtime: None,
        }
    }

    /// Build (once) and hand back a cloneable InnerTube client. Errors are
    /// surfaced so callers can report a helpful message instead of silently
    /// producing empty results.
    ///
    /// If the configured cookies.txt changes after the client was built, the
    /// client is rebuilt so fresh cookies take effect immediately.
    async fn ensure_client(&mut self) -> Result<Innertube, String> {
        let cookie_mtime = self
            .cookie_file
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()));
        if let Some(c) = &self.client
            && cookie_mtime == self.client_cookie_mtime
        {
            return Ok(c.clone());
        }
        let cookie_args = self.cookie_args();
        let options = SessionOptions {
            cookie: cookie_args,
            ..Default::default()
        };
        let client = Innertube::with_options(options)
            .await
            .map_err(|e| format!("failed to initialize InnerTube: {e}"))?;
        self.client = Some(client.clone());
        self.client_cookie_mtime = cookie_mtime;
        Ok(client)
    }

    pub fn set_cookie_file(&mut self, path: Option<String>) {
        self.cookie_file = path.map(PathBuf::from);
        // Cookie auth may have changed: force a client rebuild on next use.
        self.client = None;
        self.client_cookie_mtime = None;
    }

    /// Active cookie file path, if configured.  Callers that download audio
    /// themselves reuse this so credentials stay consistent across every
    /// extraction path.
    pub fn cookie_file(&self) -> Option<String> {
        self.cookie_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }

    async fn start_impl(&mut self, query: &str, _filter: Option<YTFilter>) -> Result<u64, String> {
        self.cancel_current();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.current_gen = generation;
        self.last_query = query.to_string();
        while self.results_rx.try_recv().is_ok() {}

        let client = self.ensure_client().await?;
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
            let results = run_search(&client, &q, cancel_rx, permit).await;
            let _ = res_tx.send((generation, results));
        });
        self.active_task = Some(handle);
        Ok(generation)
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
        self.start_impl(query, filter).await?;
        if let Some(handle) = self.active_task.take() {
            let _ = handle.await;
        }
        Ok(())
    }

    /// Fire-and-forget search: cancels any in-flight search and kicks a new
    /// one off without waiting. The IPC handler uses this so the client's
    /// short response timeout is never hit.
    pub async fn start_search(
        &mut self,
        query: &str,
        filter: Option<YTFilter>,
    ) -> Result<(), String> {
        self.start_impl(query, filter).await?;
        Ok(())
    }

    pub async fn poll_results(&mut self) -> Result<Option<(String, Vec<YTSearchResult>)>, String> {
        let mut latest: Option<(u64, Vec<YTSearchResult>)> = None;
        while let Ok(entry) = self.results_rx.try_recv() {
            latest = Some(entry);
        }
        match latest {
            Some((generation, results)) if generation == self.current_gen => {
                let query = self.last_query.clone();
                Ok(Some((query, results)))
            }
            _ => Ok(None),
        }
    }

    pub async fn cancel(&mut self) {
        self.cancel_current();
    }

    /// Start downloading a YouTube video as audio. Returns a download ID for polling.
    pub async fn download(
        &mut self,
        url: String,
        title: Option<String>,
        _artist: Option<String>,
    ) -> Result<u64, String> {
        self.cancel_download().await;

        let download_id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.download_cancel = Some(cancel_tx);

        let cookie_file = self.cookie_file.clone();
        let download_dir = self.download_dir.clone();
        let progress_tx = self.download_progress_tx.clone();

        let url_for_spawn = url.clone();
        let title_for_spawn = title.clone();

        let handle = tokio::spawn(async move {
            let progress = DownloadProgress {
                id: download_id,
                url: url_for_spawn.clone(),
                title: title_for_spawn
                    .clone()
                    .unwrap_or_else(|| url_for_spawn.clone()),
                progress: 0.0,
                status: DownloadStatus::Downloading,
                error: None,
                file_path: None,
            };
            let _ = progress_tx.send(progress.clone());

            let output_template = download_dir
                .join("%(title)s.%(ext)s")
                .to_string_lossy()
                .to_string();

            let mut cmd = Command::new("yt-dlp");
            cmd.args([
                "-x",
                "--audio-format",
                "m4a",
                "--audio-quality",
                "0",
                "-o",
                &output_template,
                &url_for_spawn,
            ]);

            if let Some(cookie_path) = cookie_file {
                cmd.args(["--cookies", &cookie_path.to_string_lossy()]);
            }

            let mut child = match cmd
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = progress_tx.send(DownloadProgress {
                        id: download_id,
                        url: url_for_spawn.clone(),
                        title: title_for_spawn
                            .clone()
                            .unwrap_or_else(|| url_for_spawn.clone()),
                        progress: 0.0,
                        status: DownloadStatus::Failed,
                        error: Some(format!("failed to spawn yt-dlp: {e}")),
                        file_path: None,
                    });
                    return;
                }
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let url_for_stdout = url_for_spawn.clone();
            let title_for_stdout = title_for_spawn.clone();
            let download_id_for_stdout = download_id;
            let progress_tx_stdout = progress_tx.clone();

            let stdout_task = if let Some(stdout) = stdout {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let reader = BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some(p) = parse_yt_dlp_progress(&line) {
                            let _ = progress_tx_stdout.send(DownloadProgress {
                                id: download_id_for_stdout,
                                url: url_for_stdout.clone(),
                                title: title_for_stdout
                                    .clone()
                                    .unwrap_or_else(|| url_for_stdout.clone()),
                                progress: p,
                                status: DownloadStatus::Downloading,
                                error: None,
                                file_path: None,
                            });
                        }
                    }
                })
            } else {
                tokio::spawn(async {})
            };

            let url_for_stderr = url_for_spawn.clone();
            let title_for_stderr = title_for_spawn.clone();
            let download_id_for_stderr = download_id;
            let progress_tx_stderr = progress_tx.clone();

            let stderr_task = if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, BufReader};
                    let reader = BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some(p) = parse_yt_dlp_progress(&line) {
                            let _ = progress_tx_stderr.send(DownloadProgress {
                                id: download_id_for_stderr,
                                url: url_for_stderr.clone(),
                                title: title_for_stderr
                                    .clone()
                                    .unwrap_or_else(|| url_for_stderr.clone()),
                                progress: p,
                                status: DownloadStatus::Downloading,
                                error: None,
                                file_path: None,
                            });
                        }
                    }
                })
            } else {
                tokio::spawn(async {})
            };

            let url_for_select = url_for_spawn.clone();
            let title_for_select = title_for_spawn.clone();
            let download_id_for_select = download_id;
            let download_dir_for_select = download_dir.clone();
            let progress_tx_select = progress_tx.clone();

            tokio::select! {
                _ = cancel_rx => {
                    let _ = child.kill().await;
                    let _ = progress_tx_select.send(DownloadProgress {
                        id: download_id_for_select,
                        url: url_for_select.clone(),
                        title: title_for_select.clone().unwrap_or_else(|| url_for_select.clone()),
                        progress: 0.0,
                        status: DownloadStatus::Cancelled,
                        error: Some("Cancelled by user".to_string()),
                        file_path: None,
                    });
                }
                result = child.wait() => {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;

                    match result {
                        Ok(status) if status.success() => {
                            let files: Vec<_> = std::fs::read_dir(&download_dir_for_select)
                                .ok()
                                .into_iter()
                                .flat_map(|d| d.filter_map(|e| e.ok()))
                                .filter(|e| e.path().extension().map(|ext| ext == "m4a").unwrap_or(false))
                                .collect();

                            let file_path = files.into_iter()
                                .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()).unwrap_or(std::time::SystemTime::UNIX_EPOCH))
                                .map(|e| e.path().to_string_lossy().to_string());

                            if let Some(fp) = file_path {
                                let _ = progress_tx_select.send(DownloadProgress {
                                    id: download_id_for_select,
                                    url: url_for_select.clone(),
                                    title: title_for_select.clone().unwrap_or_else(|| url_for_select.clone()),
                                    progress: 1.0,
                                    status: DownloadStatus::Completed,
                                    error: None,
                                    file_path: Some(fp),
                                });
                            } else {
                                let _ = progress_tx_select.send(DownloadProgress {
                                    id: download_id_for_select,
                                    url: url_for_select.clone(),
                                    title: title_for_select.clone().unwrap_or_else(|| url_for_select.clone()),
                                    progress: 0.0,
                                    status: DownloadStatus::Failed,
                                    error: Some("Download completed but file not found".to_string()),
                                    file_path: None,
                                });
                            }
                        }
                        Ok(status) => {
                            let _ = progress_tx_select.send(DownloadProgress {
                                id: download_id_for_select,
                                url: url_for_select.clone(),
                                title: title_for_select.clone().unwrap_or_else(|| url_for_select.clone()),
                                progress: 0.0,
                                status: DownloadStatus::Failed,
                                error: Some(format!("yt-dlp exited with status: {status}")),
                                file_path: None,
                            });
                        }
                        Err(e) => {
                            let url_for_err = url_for_select.clone();
                            let title_for_err = title_for_select.clone();
                            let title_resolved = title_for_err.clone().unwrap_or_else(|| url_for_err.clone());
                            let _ = progress_tx_select.send(DownloadProgress {
                                id: download_id_for_select,
                                url: url_for_err,
                                title: title_resolved,
                                progress: 0.0,
                                status: DownloadStatus::Failed,
                                error: Some(format!("yt-dlp error: {e}")),
                                file_path: None,
                            });
                        }
                    }
                }
            }
        });

        self.download_task = Some(handle);
        Ok(download_id)
    }

    /// Poll for download progress updates.
    pub fn poll_download(&mut self) -> Result<Option<DownloadProgress>, String> {
        match self.download_progress_rx.try_recv() {
            Ok(progress) => Ok(Some(progress)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err("download channel disconnected".to_string())
            }
        }
    }

    /// Cancel the current download.
    pub async fn cancel_download(&mut self) {
        if let Some(tx) = self.download_cancel.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.download_task.take() {
            handle.abort();
        }
        while self.download_progress_rx.try_recv().is_ok() {}
    }

    pub fn set_max_concurrent_downloads(&mut self, max: usize) {
        self.max_concurrent_downloads = max.max(1);
    }

    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    /// Resolve a YouTube watch URL into a playable direct audio stream.
    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo, String> {
        let client = self.ensure_client().await?;
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("semaphore: {e}"))?;
        let video_id = extract_video_id(url)?;
        let format = timeout(
            SEARCH_TIMEOUT,
            client.get_streaming_data(&video_id, &audio_filter()),
        )
        .await
        .map_err(|_| "resolve timeout".to_string())?
        .map_err(|e| format!("innertube resolve: {e}"))?;

        let url = format
            .url
            .clone()
            .ok_or_else(|| "resolved stream had no URL".to_string())?;
        let title = client
            .get_video_info(&video_id)
            .await
            .ok()
            .and_then(|i| i.video_details)
            .map(|d| d.title)
            .unwrap_or_else(|| url.to_string());

        Ok(StreamInfo {
            url,
            title,
            ext: container_ext(&format).to_string(),
            duration: format
                .approx_duration_ms
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|ms| ms / 1000.0)
                .unwrap_or(0.0),
        })
    }

    /// Download a resolved stream to a local file, streaming with reqwest
    /// (no ffmpeg transcode — the player decodes m4a/webm/opus natively).
    /// The optional cookie header is attached so authenticated streams don't
    /// get rejected with HTTP 403 the way anonymous requests do.
    pub async fn download_to_path(
        url: &str,
        dest: &Path,
        cookie_header: Option<String>,
    ) -> Result<(), String> {
        let mut req = reqwest::Client::builder()
            .user_agent(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                 Chrome/124.0 Safari/537.36",
            )
            .build()
            .map_err(|e| format!("build http client: {e}"))?
            .get(url);
        if let Some(header) = cookie_header {
            req = req.header("Cookie", header);
        }
        let resp = req.send().await.map_err(|e| format!("download: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed: HTTP {}", resp.status()));
        }

        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| format!("create {}: {e}", dest.display()))?;
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            use tokio::io::AsyncWriteExt;
            let chunk = chunk.map_err(|e| format!("download read: {e}"))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("write {}: {e}", dest.display()))?;
        }
        Ok(())
    }

    /// Cookie header resolved from the configured cookies.txt (Netscape) file,
    /// if any. Passed to the InnerTube session so authenticated extraction
    /// works (YouTube answers anonymous requests with HTTP 403).
    fn cookie_args(&self) -> Option<String> {
        let cookies = match self.cookie_file.as_ref() {
            Some(p) if p.is_file() => parse_cookie_file(p),
            _ => return None,
        };
        let refs: Vec<&Cookie> = cookies.iter().collect();
        header_from_cookies(&refs)
    }

    /// Cookie header for a specific download URL, domain-filtered so unrelated
    /// cookies in the file aren't sent to the CDN. Falls back to the full
    /// cookie set (a bare YouTube export is almost always what's configured)
    /// so authenticated streams — the typical HTTP 403 case — are covered.
    pub fn cookie_header_for(&self, url: &str) -> Option<String> {
        let cookies = match self.cookie_file.as_ref() {
            Some(p) if p.is_file() => parse_cookie_file(p),
            _ => return None,
        };
        header_for_host(&cookies, url)
    }
}

/// Build the tuned audio-only format filter used across searches/downloads.
fn audio_filter() -> FormatFilter {
    FormatFilter {
        format_type: FormatType::AudioOnly,
        quality: QualityPreference::Highest,
        container: None,
    }
}

/// Run one InnerTube search to completion, mapping results into
/// [`YTSearchResult`]s until the caller cancels via `cancel_rx` or
/// [`SEARCH_TIMEOUT`] elapses.
async fn run_search(
    client: &Innertube,
    query: &str,
    cancel_rx: oneshot::Receiver<()>,
    _permit: OwnedSemaphorePermit,
) -> Vec<YTSearchResult> {
    let search_arg = if query.starts_with("http://") || query.starts_with("https://") {
        query.to_string()
    } else {
        // Fine-tune the query so the first hits are single tracks, mirroring
        // the old `ytsearch10:<query> official audio` behaviour.
        format!("{query} official audio")
    };

    let search = async {
        timeout(SEARCH_TIMEOUT, client.search(&search_arg, None))
            .await
            .map_err(|_| "search timeout".to_string())
            .and_then(|r| r.map_err(|e| format!("innertube search: {e}")))
    };

    let results = match tokio::select! {
        res = search => res,
        _ = cancel_rx => return Vec::new(),
    } {
        Ok(results) => results,
        Err(e) => {
            debug!("{e}");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for item in results.items {
        match item {
            innertube_rs::SearchResultItem::Video(v) => {
                if let Some(r) = parse_video(&v) {
                    out.push(r);
                }
            }
            innertube_rs::SearchResultItem::Playlist(p) => {
                out.push(YTSearchResult {
                    id: p.playlist_id.clone(),
                    title: p.title,
                    url: format!("https://www.youtube.com/playlist?list={}", p.playlist_id),
                    channel: p.author,
                    duration: 0.0,
                    views: 0,
                    thumbnail: p.thumbnails.first().map(|t| t.url.clone()),
                    is_playlist: true,
                    artist: None,
                    priority: 0,
                });
            }
            innertube_rs::SearchResultItem::Channel(_) => {}
        }
    }
    out.sort_by(|a, b| b.priority.cmp(&a.priority).then(b.views.cmp(&a.views)));
    out
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

fn parse_video(v: &innertube_rs::SearchVideoItem) -> Option<YTSearchResult> {
    let raw_title = v.title.clone();
    let (artist, title) = crate::cleaner::clean_youtube_title(&raw_title);
    Some(YTSearchResult {
        id: v.video_id.clone(),
        title,
        url: format!("https://www.youtube.com/watch?v={}", v.video_id),
        channel: v.author.clone(),
        duration: v
            .duration
            .as_deref()
            .and_then(parse_duration)
            .unwrap_or(0.0),
        views: v
            .view_count
            .as_deref()
            .and_then(parse_view_count)
            .unwrap_or(0),
        thumbnail: v.thumbnails.first().map(|t| t.url.clone()),
        is_playlist: false,
        artist,
        priority: priority(&raw_title),
    })
}

/// "1:02:33" / "5:30" → seconds. Handles the `HH:MM:SS` / `MM:SS` formats
/// YouTube returns on search results.
fn parse_duration(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let n: f64 = part.parse().ok()?;
        total += n * 60f64.powi((parts.len() - 1 - i) as i32);
    }
    Some(total)
}

/// "12,345,678 views" → 12345678.
fn parse_view_count(s: &str) -> Option<u64> {
    let cleaned: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .collect();
    let trimmed = cleaned.replace(',', "");
    trimmed.parse().ok()
}

/// Extract the 11-char video id from a YouTube watch/shorts/playlist URL.
fn extract_video_id(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|_| format!("invalid URL: {url}"))?;
    if parsed.host_str() == Some("youtu.be")
        && let Some(id) = parsed.path_segments().and_then(|mut s| s.next())
        && id.len() == 11
    {
        return Ok(id.to_string());
    }
    if let Some(id) = parsed
        .query_pairs()
        .find(|(k, _)| k == "v")
        .map(|(_, v)| v.to_string())
        .filter(|id| id.len() == 11)
    {
        return Ok(id);
    }
    // Playlist/watch URLs also embed the id in the path (e.g. /playlist?list=).
    Err(format!("could not determine video id from: {url}"))
}

/// Map an innertube mime_type ("audio/mp4", "audio/webm; codecs=opus") to a
/// file extension.
fn container_ext(f: &StreamingFormat) -> &'static str {
    let mime = f.mime_type.split(';').next().unwrap_or("");
    match mime {
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/webm" => "webm",
        "audio/opus" => "opus",
        "audio/ogg" => "ogg",
        _ => "m4a",
    }
}

/// Parse a Netscape cookies.txt file into individual cookies, keeping the
/// domain so headers can be scoped to the exact host being requested.
fn parse_cookie_file(path: &Path) -> Vec<Cookie> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut cookies: Vec<Cookie> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let name = fields[5];
        let value = fields[6];
        if !name.is_empty() && !value.is_empty() {
            cookies.push(Cookie {
                domain: fields[0].to_string(),
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
    cookies
}

/// Join cookies into a single `name=value; name=value` header string.
fn header_from_cookies(cookies: &[&Cookie]) -> Option<String> {
    if cookies.is_empty() {
        return None;
    }
    Some(
        cookies
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// True when a cookie's domain applies to `host` (domain parent matching,
/// Netscape cookies use a leading dot to signal "all subdomains").
fn cookie_matches(domain: &str, host: &str) -> bool {
    let d = domain.trim_start_matches('.');
    host == d || host.ends_with(&format!(".{d}"))
}

/// Header for `host` only (plus the all-cookies fallback when nothing
/// belongs to that host, e.g. a bare YouTube export hitting googlevideo).
fn header_for_host(cookies: &[Cookie], url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_string();
    let mut matching: Vec<&Cookie> = cookies
        .iter()
        .filter(|c| cookie_matches(&c.domain, &host))
        .collect();
    if matching.is_empty() {
        matching = cookies.iter().collect();
    }
    header_from_cookies(&matching)
}
