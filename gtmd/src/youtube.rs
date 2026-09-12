// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// YouTube search via the InnerTube API (innertube-rs), with stream
// resolution and audio download delegated to the yt-dlp subprocess.
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use innertube_rs::{Innertube, SessionOptions};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::debug;

use gtm_core::global::YTFilter;
use gtm_core::track::{StreamInfo, YTSearchResult};

use crate::cleaner::clean_youtube_title;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT: usize = 2;

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
    /// Browser cookies source forwarded to yt-dlp as `--cookies-from-browser`
    /// (e.g. `chrome`, `firefox`, `brave`). Takes precedence over `cookie_file`.
    cookie_source: Option<String>,
    /// JS interpreter forwarded to yt-dlp as `--js-runtime`.
    js_runtime: Option<String>,
    generation: Arc<AtomicU64>,
    current_gen: u64,
    last_query: String,
    results_tx: mpsc::UnboundedSender<(u64, Vec<YTSearchResult>)>,
    results_rx: mpsc::UnboundedReceiver<(u64, Vec<YTSearchResult>)>,
    download_task: Option<JoinHandle<()>>,
    download_cancel: Option<oneshot::Sender<()>>,
    download_progress_tx: mpsc::UnboundedSender<DownloadProgress>,
    download_progress_rx: mpsc::UnboundedReceiver<DownloadProgress>,
    playlist_task: Option<JoinHandle<()>>,
    playlist_cancel: Option<oneshot::Sender<()>>,
    playlist_tx: mpsc::UnboundedSender<(String, Vec<YTSearchResult>)>,
    playlist_rx: mpsc::UnboundedReceiver<(String, Vec<YTSearchResult>)>,
    download_dir: PathBuf,
    max_concurrent_downloads: usize,
    /// Mtime of the cookie file the current InnerTube client was built with,
    /// so freshly-exported cookies (the usual fix for HTTP 403) are picked up
    /// without requiring a restart.
    client_cookie_mtime: Option<std::time::SystemTime>,
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
    /// Bytes written so far, when yt-dlp reports a size.
    pub downloaded_bytes: Option<u64>,
    /// Total size to download, when known.
    pub total_bytes: Option<u64>,
    /// Current transfer rate in bytes/second.
    pub rate_bytes_per_sec: Option<f64>,
    /// Remaining seconds per yt-dlp's ETA line.
    pub eta_secs: Option<u64>,
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
        let (playlist_tx, playlist_rx) = mpsc::unbounded_channel();
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
            cookie_source: None,
            js_runtime: None,
            generation: Arc::new(AtomicU64::new(0)),
            current_gen: 0,
            last_query: String::new(),
            results_tx,
            results_rx,
            download_task: None,
            download_cancel: None,
            download_progress_tx,
            download_progress_rx,
            playlist_task: None,
            playlist_cancel: None,
            playlist_tx,
            playlist_rx,
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
        let cookie_args = self
            .cookie_file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
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

    pub fn set_cookie_source(&mut self, source: Option<String>) {
        self.cookie_source = source;
    }

    pub fn set_js_runtime(&mut self, runtime: Option<String>) {
        self.js_runtime = runtime;
    }

    /// Extra `yt-dlp` arguments for the configured auth/cookie setup:
    /// `--cookies-from-browser <source>` when a browser source is set (it takes
    /// precedence over a cookie file), otherwise `--cookies <file>`, plus
    /// `--js-runtime` when configured. yt-dlp refuses both `--cookies` forms at
    /// the same time, so at most one cookie flag is emitted.
    fn ytdlp_auth_args(&self) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        if let Some(source) = &self.cookie_source {
            args.push("--cookies-from-browser".into());
            args.push(source.as_str().into());
        } else if let Some(path) = &self.cookie_file {
            args.push("--cookies".into());
            args.push(path.as_os_str().into());
        }
        if let Some(runtime) = &self.js_runtime {
            args.push("--js-runtime".into());
            args.push(runtime.as_str().into());
        }
        args
    }

    /// Copy of [`Self::ytdlp_auth_args`] for callers that need the flags while
    /// no longer holding the manager lock (e.g. the Spotify YouTube fallback
    /// which downloads audio into its own cache).
    pub fn auth_args(&self) -> Vec<std::ffi::OsString> {
        self.ytdlp_auth_args()
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

    /// Start downloading a YouTube video as audio via yt-dlp. Returns a
    /// download ID for polling with [`YoutubeManager::poll_download`]. yt-dlp
    /// re-extracts fresh stream URLs (PO token + signature) on every run, so
    /// the negotiated CDN URLs are always current instead of going stale and
    /// answering HTTP 403.
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

        let auth_args = self.ytdlp_auth_args();
        let download_dir = self.download_dir.clone();
        let progress_tx = self.download_progress_tx.clone();

        let url_for_spawn = url.clone();
        let title_for_spawn = title.clone();

        let handle = tokio::spawn(async move {
            let name = title_for_spawn
                .clone()
                .unwrap_or_else(|| url_for_spawn.clone());
            let _ = progress_tx.send(DownloadProgress {
                id: download_id,
                url: url_for_spawn.clone(),
                title: name.clone(),
                progress: 0.0,
                status: DownloadStatus::Downloading,
                error: None,
                file_path: None,
                downloaded_bytes: None,
                total_bytes: None,
                rate_bytes_per_sec: None,
                eta_secs: None,
            });

            let output_template = download_dir
                .join("%(title)s.%(ext)s")
                .to_string_lossy()
                .to_string();

            let mut args: Vec<std::ffi::OsString> = vec![
                "-f".into(),
                "bestaudio[ext=m4a]/bestaudio".into(),
                "--no-playlist".into(),
                "--newline".into(),
                "-o".into(),
                output_template.into(),
            ];
            args.extend(auth_args.iter().cloned());
            args.push(url_for_spawn.clone().into());

            let mut child = match Command::new("yt-dlp")
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = progress_tx.send(DownloadProgress {
                        id: download_id,
                        url: url_for_spawn.clone(),
                        title: name,
                        progress: 0.0,
                        status: DownloadStatus::Failed,
                        error: Some(format!("failed to spawn yt-dlp: {e}")),
                        file_path: None,
                        downloaded_bytes: None,
                        total_bytes: None,
                        rate_bytes_per_sec: None,
                        eta_secs: None,
                    });
                    return;
                }
            };

            let stdout_task = child
                .stdout
                .take()
                .map(|s| {
                    tokio::spawn(spawn_progress_reader(
                        s,
                        progress_tx.clone(),
                        download_id,
                        url_for_spawn.clone(),
                        name.clone(),
                    ))
                })
                .unwrap_or_else(|| tokio::spawn(async {}));
            let stderr_task = child
                .stderr
                .take()
                .map(|s| {
                    tokio::spawn(spawn_progress_reader(
                        s,
                        progress_tx.clone(),
                        download_id,
                        url_for_spawn.clone(),
                        name.clone(),
                    ))
                })
                .unwrap_or_else(|| tokio::spawn(async {}));

            let outcome = tokio::select! {
                _ = cancel_rx => {
                    let _ = child.kill().await;
                    (None, "cancelled".to_string())
                }
                result = child.wait() => {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    match result {
                        Ok(status) if status.success() => {
                            let file = std::fs::read_dir(&download_dir)
                                .ok()
                                .into_iter()
                                .flatten()
                                .filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .filter(|p| {
                                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                                    let file_name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    matches!(ext.as_str(), "m4a" | "webm" | "opus" | "mp3" | "ogg" | "aac" | "flac" | "wav")
                                        && !file_name.ends_with(".part")
                                })
                                .max_by_key(|p| p.metadata().ok().and_then(|m| m.modified().ok()));
                            (file, String::new())
                        }
                        Ok(status) => (None, format!("yt-dlp exited with {status}")),
                        Err(e) => (None, format!("yt-dlp error: {e}")),
                    }
                }
            };

            let (maybe_file, err) = outcome;
            let status = if maybe_file.is_some() {
                DownloadStatus::Completed
            } else if err == "cancelled" {
                DownloadStatus::Cancelled
            } else {
                DownloadStatus::Failed
            };
            let _ = progress_tx.send(DownloadProgress {
                id: download_id,
                url: url_for_spawn.clone(),
                title: name,
                progress: if maybe_file.is_some() { 1.0 } else { 0.0 },
                status,
                error: if maybe_file.is_none() && err != "cancelled" {
                    Some(err)
                } else {
                    None
                },
                downloaded_bytes: maybe_file.is_some().then_some(0u64),
                total_bytes: maybe_file.is_some().then_some(0u64),
                rate_bytes_per_sec: None,
                eta_secs: None,
                file_path: maybe_file.map(|p| p.to_string_lossy().into_owned()),
            });
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

    pub fn set_download_dir(&mut self, dir: Option<String>) {
        if let Some(dir) = dir {
            let path = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&path);
            self.download_dir = path;
        }
    }

    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    /// Fire-and-forget playlist entry fetch via `yt-dlp --flat-playlist
    /// --dump-json`. Results are collected with
    /// [`YoutubeManager::poll_playlist`]; a new fetch cancels any in-flight
    /// one. Runs under the shared semaphore so it never competes unfairly with
    /// stream resolution.
    pub fn start_fetch_playlist(&mut self, url: String) -> Result<(), String> {
        self.cancel_playlist_fetch();

        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.playlist_cancel = Some(cancel_tx);

        let auth_args = self.ytdlp_auth_args();
        let semaphore = self.semaphore.clone();
        let res_tx = self.playlist_tx.clone();
        let handle = tokio::spawn(async move {
            let permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let results = run_playlist_fetch(&url, auth_args, cancel_rx, permit).await;
            let _ = res_tx.send((url, results));
        });
        self.playlist_task = Some(handle);
        Ok(())
    }

    /// Drain any finished playlist fetch, returning the source URL and the
    /// entries published since the last poll.
    pub fn poll_playlist(&mut self) -> Result<Option<(String, Vec<YTSearchResult>)>, String> {
        match self.playlist_rx.try_recv() {
            Ok(entry) => Ok(Some(entry)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err("playlist channel disconnected".to_string())
            }
        }
    }

    fn cancel_playlist_fetch(&mut self) {
        if let Some(tx) = self.playlist_cancel.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.playlist_task.take() {
            handle.abort();
        }
        while self.playlist_rx.try_recv().is_ok() {}
    }

    /// Resolve a YouTube watch URL into a playable direct audio stream using
    /// yt-dlp's maintained extractor (fresh PO tokens and signature handling),
    /// so the returned CDN URL is not a stale, HTTP-403'd one.
    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo, String> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("semaphore: {e}"))?;

        let mut args: Vec<std::ffi::OsString> = vec![
            "-g".into(),
            "-f".into(),
            "bestaudio[ext=m4a]/bestaudio".into(),
        ];
        args.extend(self.ytdlp_auth_args());
        args.push(url.to_string().into());

        let output = timeout(SEARCH_TIMEOUT, Command::new("yt-dlp").args(&args).output())
            .await
            .map_err(|_| "resolve timeout".to_string())?
            .map_err(|e| format!("yt-dlp: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("unknown error");
            let hint = if detail.contains("403") || detail.contains("Forbidden") {
                " (try setting a cookie file in Settings → YouTube)"
            } else {
                ""
            };
            return Err(format!("yt-dlp resolve failed: {detail}{hint}"));
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

/// Download a YouTube URL into `dest_dir` under `prefix.<ext>` using yt-dlp,
/// so authenticated/PO-token extraction succeeds instead of answering HTTP 403.
/// Returns the path of the produced audio file.
pub(crate) async fn download_into(
    url: &str,
    dest_dir: &Path,
    prefix: &str,
    auth: &[std::ffi::OsString],
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("create download dir: {e}"))?;
    let template = dest_dir
        .join(format!("{prefix}.%(ext)s"))
        .to_string_lossy()
        .into_owned();

    let mut args: Vec<std::ffi::OsString> = vec![
        "-f".into(),
        "bestaudio[ext=m4a]/bestaudio".into(),
        "--no-playlist".into(),
        "-o".into(),
        template.into(),
    ];
    args.extend(auth.iter().cloned());
    args.push(url.to_string().into());

    let output = timeout(
        Duration::from_secs(180),
        Command::new("yt-dlp").args(&args).output(),
    )
    .await
    .map_err(|_| "download timed out".to_string())?
    .map_err(|e| format!("yt-dlp: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        let hint = if detail.contains("403") || detail.contains("Forbidden") {
            " (try setting a cookie file in Settings → YouTube)"
        } else {
            ""
        };
        return Err(format!("yt-dlp download failed: {detail}{hint}"));
    }

    let produced = std::fs::read_dir(dest_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            name.starts_with(prefix) && !name.ends_with(".part")
        })
        .max_by_key(|p| p.metadata().ok().and_then(|m| m.modified().ok()));
    match produced {
        Some(path) => Ok(path),
        None => Err("yt-dlp produced no file".to_string()),
    }
}

/// Parse `[download] 42.3% of ...` progress lines emitted by yt-dlp with
/// `--newline` and publish them on the download progress channel.
async fn spawn_progress_reader<R>(
    reader: R,
    progress_tx: mpsc::UnboundedSender<DownloadProgress>,
    download_id: u64,
    url: String,
    title: String,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(fields) = parse_yt_dlp_progress(&line) {
            let _ = progress_tx.send(DownloadProgress {
                id: download_id,
                url: url.clone(),
                title: title.clone(),
                progress: fields.percent,
                status: DownloadStatus::Downloading,
                error: None,
                file_path: None,
                downloaded_bytes: fields.downloaded_bytes,
                total_bytes: fields.total_bytes,
                rate_bytes_per_sec: fields.rate_bytes_per_sec,
                eta_secs: fields.eta_secs,
            });
        }
    }
}

struct YtDlpLine {
    percent: f64,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    rate_bytes_per_sec: Option<f64>,
    eta_secs: Option<u64>,
}

/// Parse a yt-dlp progress line like
/// `[download]  42.3% of 3.86MiB at 1.10MiB/s ETA 00:00` and return the fields
/// we track. Sizes and rates use the exact units yt-dlp prints (`KiB/MiB/GiB`,
/// occasionally with a `~` for estimated totals), so they convert cleanly to
/// bytes.
fn parse_yt_dlp_progress(line: &str) -> Option<YtDlpLine> {
    let start = line.find('%')?;
    let prefix = &line[..start];
    let digits = prefix
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let percent: f64 = digits.parse().ok()?;
    Some(YtDlpLine {
        percent,
        downloaded_bytes: extract_downloaded_bytes(line, start),
        total_bytes: extract_total_bytes(line),
        rate_bytes_per_sec: extract_rate(line),
        eta_secs: extract_eta(line),
    })
}

/// `of 3.86MiB` — the part right after the `%`. The downloaded byte count is
/// derived from the reported percent once the total is known.
fn extract_downloaded_bytes(line: &str, percent_start: usize) -> Option<u64> {
    let percent = parse_num_before(&line[..percent_start]);
    extract_total_bytes(line).map(|total| (percent / 100.0 * total as f64) as u64)
}

fn parse_num_before(s: &str) -> f64 {
    s.chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0.0)
}

fn extract_total_bytes(line: &str) -> Option<u64> {
    let idx = line.find("of ")?;
    parse_size(&line[idx + 3..])
}

fn extract_rate(line: &str) -> Option<f64> {
    let idx = line.find(" at ")?;
    let tail = &line[idx + 4..];
    let digits: String = tail
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if digits.is_empty() {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    let unit = unit_token(&tail[digits.len()..]);
    Some(value * unit_multiplier(unit)?)
}

fn extract_eta(line: &str) -> Option<u64> {
    let idx = line.rfind(" ETA ")?;
    let eta: String = line[idx + 5..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let mut parts: Vec<u64> = eta.split(':').filter_map(|p| p.parse().ok()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut secs = parts.pop()?;
    let mut factor = 60u64;
    while let Some(part) = parts.pop() {
        secs += part * factor;
        factor *= 60;
    }
    Some(secs)
}

/// Parse a size token at the beginning of `s`, e.g. `3.86MiB` or `~2.36GiB`
/// (yt-dlp uses `~` to mark estimated totals).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim_start();
    let s = s.strip_prefix('~').unwrap_or(s);
    let digits: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if digits.is_empty() {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    Some((value * unit_multiplier(unit_token(&s[digits.len()..]))?) as u64)
}

/// Leading alphabetic run of `s` — the unit part of a size/rate token.
fn unit_token(s: &str) -> &str {
    let n = s.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    &s[..n]
}

fn unit_multiplier(unit: &str) -> Option<f64> {
    match unit.to_ascii_lowercase().as_str() {
        "b" => Some(1.0),
        "kib" | "kb" => Some(1024.0),
        "mib" | "mb" => Some(1024.0 * 1024.0),
        "gib" | "gb" => Some(1024.0 * 1024.0 * 1024.0),
        "tib" | "tb" => Some(1024.0 * 1024.0 * 1024.0 * 1024.0),
        _ => None,
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

/// Collect the entries of a YouTube playlist/URL via `yt-dlp --flat-playlist
/// --dump-json`, mapping each JSON-lines record into a [`YTSearchResult`]
/// until the caller cancels via `cancel_rx` or [`SEARCH_TIMEOUT`] elapses.
async fn run_playlist_fetch(
    url: &str,
    auth_args: Vec<std::ffi::OsString>,
    cancel_rx: oneshot::Receiver<()>,
    _permit: OwnedSemaphorePermit,
) -> Vec<YTSearchResult> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--flat-playlist".into(),
        "--dump-json".into(),
        "--no-warnings".into(),
    ];
    args.extend(auth_args);
    args.push(url.to_string().into());

    let fetch = async {
        let output =
            match timeout(SEARCH_TIMEOUT, Command::new("yt-dlp").args(&args).output()).await {
                Ok(res) => res.map_err(|e| format!("yt-dlp: {e}"))?,
                Err(_) => return Err("playlist fetch timeout".to_string()),
            };
        if output.status.success() {
            Ok(output.stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("unknown error");
            Err(format!("yt-dlp playlist fetch: {detail}"))
        }
    };

    let stdout = match tokio::select! {
        res = fetch => res,
        _ = cancel_rx => return Vec::new(),
    } {
        Ok(out) => out,
        Err(e) => {
            debug!("{e}");
            return Vec::new();
        }
    };

    let mut results = Vec::new();
    for line in stdout.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_slice::<Value>(line)
            && let Some(r) = parse_flat_entry(&entry)
        {
            results.push(r);
        }
    }
    results
}

/// Map a `yt-dlp --flat-playlist --dump-json` record into a
/// [`YTSearchResult`]. Flat entries carry `id`/`title`/`channel` but usually
/// no duration; the URL is reconstructed from the id so playback goes through
/// the normal resolve path.
fn parse_flat_entry(v: &Value) -> Option<YTSearchResult> {
    let id = v.get("id").and_then(|i| i.as_str())?.to_string();
    if id.is_empty() {
        return None;
    }
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if title.is_empty() {
        return None;
    }
    let channel = ["channel", "uploader", "playlist_uploader"]
        .iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    let thumbnail = v
        .get("thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());
    let duration = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
    let url = format!("https://www.youtube.com/watch?v={id}");
    Some(YTSearchResult {
        id,
        title,
        url,
        channel,
        duration,
        views: 0,
        thumbnail,
        is_playlist: false,
        artist: None,
        priority: 0,
    })
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
    let (artist, title) = clean_youtube_title(&raw_title);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_line_extracts_fields() {
        let line = "[download]  42.3% of 3.86MiB at 1.10MiB/s ETA 00:07";
        let fields = parse_yt_dlp_progress(line).expect("progress line parses");
        assert!((fields.percent - 42.3).abs() < 1e-9);
        assert_eq!(fields.total_bytes, Some((3.86 * 1024.0 * 1024.0) as u64));
        assert!((fields.rate_bytes_per_sec.unwrap() - 1.10 * 1024.0 * 1024.0).abs() < 1.0);
        assert_eq!(fields.eta_secs, Some(7));
        assert_eq!(
            fields.downloaded_bytes,
            Some((0.423 * fields.total_bytes.unwrap() as f64) as u64)
        );
    }

    #[test]
    fn parse_progress_estimated_total_and_mmss_eta() {
        let line = "[download]   5.0% of ~2.36GiB at 90.5KiB/s ETA 07:10";
        let fields = parse_yt_dlp_progress(line).expect("estimated total parses");
        assert!((fields.percent - 5.0).abs() < 1e-9);
        let gi = 1024.0 * 1024.0 * 1024.0;
        assert_eq!(fields.total_bytes, Some((2.36 * gi) as u64));
        assert!((fields.rate_bytes_per_sec.unwrap() - 90.5 * 1024.0).abs() < 1.0);
        assert_eq!(fields.eta_secs, Some(7 * 60 + 10));
    }

    #[test]
    fn parse_progress_does_not_parse_non_progress_lines() {
        assert!(parse_yt_dlp_progress("[download] Destination: /tmp/x.m4a").is_none());
        assert!(parse_yt_dlp_progress("[ExtractAudio] Destination: /tmp/x.m4a").is_none());
    }
}
