// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// YouTube search via the InnerTube API (innertube-rs), with stream
// resolution and audio download delegated to the yt-dlp subprocess.
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use innertube_rs::{Innertube, SessionOptions};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::debug;

use gtm::shared::global::YTFilter;
use gtm::shared::track::{StreamInfo, YTSearchResult};
use gtm::shared::yt::{match_yt_host, yt_hosts};

use crate::cleaner::clean_youtube_title;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT: usize = 2;

/// Minimum gap enforced between yt-dlp subprocess launches. YouTube treats a
/// rapid succession of extractor runs (stream resolve + download + search +
/// playlist fetch) as a request burst and answers HTTP 429, which is exactly
/// the "failed to stream or download the selected track" symptom. Every
/// yt-dlp invocation site shares this launch gate, so subprocesses start at
/// least this far apart even when several tasks fire at once.
const YTDLP_MIN_LAUNCH_GAP: Duration = Duration::from_millis(1500);
/// A rate-limited (HTTP 429 / "Too Many Requests") yt-dlp run is retried this
/// many times with exponential backoff before the failure is surfaced.
const YTDLP_RATE_LIMIT_RETRIES: usize = 2;
/// Base backoff for the first rate-limit retry (doubled per attempt).
const YTDLP_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(6);

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
    /// Shared yt-dlp launch gate (see [`YTDLP_MIN_LAUNCH_GAP`]). Cloned into
    /// spawned tasks and handed to the daemon so every yt-dlp site — resolve,
    /// download, search, playlist fetch — goes through the same throttle.
    last_ytdlp_launch: Arc<Mutex<Instant>>,
    cookie_file: Option<PathBuf>,
    /// Browser cookies source forwarded to yt-dlp as `--cookies-from-browser`
    /// (e.g. `chrome`, `firefox`, `brave`). Takes precedence over `cookie_file`.
    /// When neither is configured, a browser is auto-detected from its standard
    /// cookie location (see [`detect_browser_cookie_source`]).
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
    pub rate_bps: Option<f64>,
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

/// Chromium-family cookie DB locations under `~/.config` (Linux). Each row is
/// `(yt-dlp browser name, path relative to the config dir)`. The store moved
/// from `Default/Cookies` to `Default/Network/Cookies` in newer versions, so
/// both are probed per browser.
#[cfg(target_os = "linux")]
const LINUX_BROWSER_COOKIES: &[(&str, &str)] = &[
    ("chrome", "google-chrome/Default/Network/Cookies"),
    ("chrome", "google-chrome/Default/Cookies"),
    ("chromium", "chromium/Default/Network/Cookies"),
    ("chromium", "chromium/Default/Cookies"),
    (
        "brave",
        "BraveSoftware/Brave-Browser/Default/Network/Cookies",
    ),
    ("brave", "BraveSoftware/Brave-Browser/Default/Cookies"),
    ("edge", "microsoft-edge/Default/Network/Cookies"),
    ("edge", "microsoft-edge/Default/Cookies"),
    ("vivaldi", "vivaldi/Default/Network/Cookies"),
    ("vivaldi", "vivaldi/Default/Cookies"),
    ("opera", "opera/Cookies"),
    ("opera", "opera/Default/Cookies"),
];

/// Chromium-family cookie DB locations under `~/Library/Application Support`
/// (macOS), plus Safari's binary cookie store which lives elsewhere.
#[cfg(target_os = "macos")]
const MACOS_BROWSER_COOKIES: &[(&str, &str)] = &[
    ("chrome", "Google/Chrome/Default/Network/Cookies"),
    ("chrome", "Google/Chrome/Default/Cookies"),
    ("chromium", "Chromium/Default/Network/Cookies"),
    ("chromium", "Chromium/Default/Cookies"),
    (
        "brave",
        "BraveSoftware/Brave-Browser/Default/Network/Cookies",
    ),
    ("brave", "BraveSoftware/Brave-Browser/Default/Cookies"),
    ("edge", "Microsoft Edge/Default/Network/Cookies"),
    ("edge", "Microsoft Edge/Default/Cookies"),
    ("vivaldi", "Vivaldi/Default/Network/Cookies"),
    ("vivaldi", "Vivaldi/Default/Cookies"),
    ("opera", "com.operasoftware.Opera/Cookies"),
    ("opera", "com.operasoftware.Opera/Default/Cookies"),
];

/// Chromium-family cookie DB locations under `%LOCALAPPDATA%` (Windows).
#[cfg(target_os = "windows")]
const WINDOWS_BROWSER_COOKIES: &[(&str, &str)] = &[
    ("chrome", "Google/Chrome/User Data/Default/Network/Cookies"),
    ("chrome", "Google/Chrome/User Data/Default/Cookies"),
    ("edge", "Microsoft/Edge/User Data/Default/Network/Cookies"),
    ("edge", "Microsoft/Edge/User Data/Default/Cookies"),
    (
        "brave",
        "BraveSoftware/Brave-Browser/User Data/Default/Network/Cookies",
    ),
    (
        "brave",
        "BraveSoftware/Brave-Browser/User Data/Default/Cookies",
    ),
    ("chromium", "Chromium/User Data/Default/Network/Cookies"),
    ("chromium", "Chromium/User Data/Default/Cookies"),
    ("vivaldi", "Vivaldi/User Data/Default/Network/Cookies"),
    ("vivaldi", "Vivaldi/User Data/Default/Cookies"),
    ("opera", "Opera Software/Opera Stable/Network/Cookies"),
    ("opera", "Opera Software/Opera Stable/Cookies"),
];

/// True when any Firefox profile under `root` ships a local cookie database.
fn profile_has_cookies(root: &Path) -> bool {
    std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .any(|e| e.path().join("cookies.sqlite").exists())
}

/// Locate the cookie store of an installed major browser in its standard
/// per-OS location. Returns the browser name yt-dlp accepts for
/// `--cookies-from-browser` (e.g. `chrome`, `brave`, `firefox`, `edge`).
#[cfg(target_os = "linux")]
fn detect_browser_cookie_source() -> Option<&'static str> {
    let home = dirs::home_dir()?;
    let base = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    for (name, rel) in LINUX_BROWSER_COOKIES {
        if base.join(rel).exists() {
            return Some(*name);
        }
    }
    profile_has_cookies(&home.join(".mozilla/firefox")).then_some("firefox")
}

#[cfg(target_os = "macos")]
fn detect_browser_cookie_source() -> Option<&'static str> {
    let home = dirs::home_dir()?;
    let base = dirs::config_dir().unwrap_or_else(|| home.join("Library/Application Support"));
    for (name, rel) in MACOS_BROWSER_COOKIES {
        if base.join(rel).exists() {
            return Some(*name);
        }
    }
    if profile_has_cookies(&base.join("Firefox/Profiles")) {
        return Some("firefox");
    }
    let safari =
        home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies");
    safari.exists().then_some("safari")
}

#[cfg(target_os = "windows")]
fn detect_browser_cookie_source() -> Option<&'static str> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    for (name, rel) in WINDOWS_BROWSER_COOKIES {
        if base.join(rel).exists() {
            return Some(*name);
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA")
        && profile_has_cookies(&PathBuf::from(appdata).join("Mozilla/Firefox/Profiles"))
    {
        return Some("firefox");
    }
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_browser_cookie_source() -> Option<&'static str> {
    None
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
            // Pre-stamped so the very first launch is not delayed.
            last_ytdlp_launch: Arc::new(Mutex::new(Instant::now() - YTDLP_MIN_LAUNCH_GAP)),
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
        let cookie_mtime = match self.cookie_file.as_ref() {
            Some(p) => tokio::fs::metadata(p)
                .await
                .ok()
                .and_then(|m| m.modified().ok()),
            None => None,
        };
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
    ///
    /// When nothing is configured, the cookie store of an installed major
    /// browser is auto-detected from its standard location and passed via
    /// `--cookies-from-browser`, so YouTube playback/downloads work out of the
    /// box instead of failing with HTTP 403.
    fn ytdlp_auth_args(&self) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        if let Some(source) = &self.cookie_source {
            args.push("--cookies-from-browser".into());
            args.push(source.as_str().into());
        } else if let Some(path) = &self.cookie_file {
            args.push("--cookies".into());
            args.push(path.as_os_str().into());
        } else if let Some(browser) = detect_browser_cookie_source() {
            args.push("--cookies-from-browser".into());
            args.push(browser.into());
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

    /// Hand the daemon everything it needs to run a yt-dlp resolve *outside*
    /// the manager lock: the auth/cookie flags, the shared concurrency permit,
    /// and the shared launch gate (all cheap clones). Resolving takes up to
    /// [`SEARCH_TIMEOUT`] (more under rate-limit backoff); holding the
    /// `YoutubeManager` mutex across that would stall unrelated requests such
    /// as `YtDownload` behind the lock.
    pub fn yt_extras(
        &self,
    ) -> (
        Vec<std::ffi::OsString>,
        Arc<Semaphore>,
        Arc<Mutex<Instant>>,
    ) {
        (
            self.ytdlp_auth_args(),
            self.semaphore.clone(),
            self.last_ytdlp_launch.clone(),
        )
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
        let gate = self.last_ytdlp_launch.clone();
        let res_tx = self.results_tx.clone();
        let q = query.to_string();
        let auth_args = self.ytdlp_auth_args();
        let handle = tokio::spawn(async move {
            let permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let results = run_search(&client, &q, auth_args, gate, cancel_rx, permit).await;
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
        let semaphore = self.semaphore.clone();
        let gate = self.last_ytdlp_launch.clone();
        // Captures the tail of yt-dlp's stderr so the failure toast can tell
        // rate-limit (429) and cookie (403) failures apart from other errors.
        let stderr_capture = Arc::new(std::sync::Mutex::new(String::new()));

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
                rate_bps: None,
                eta_secs: None,
            });

            // Join the shared yt-dlp launch queue (permit + spacing gate) so a
            // download can never pile on top of an in-flight resolve/search in
            // a burst that trips YouTube's rate limiter.
            let permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    let _ = progress_tx.send(DownloadProgress {
                        id: download_id,
                        url: url_for_spawn.clone(),
                        title: name,
                        progress: 0.0,
                        status: DownloadStatus::Failed,
                        error: Some("yt-dlp queue closed".to_string()),
                        file_path: None,
                        downloaded_bytes: None,
                        total_bytes: None,
                        rate_bps: None,
                        eta_secs: None,
                    });
                    return;
                }
            };
            gate_ytdlp_launch(&gate).await;
            let _permit = permit;

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
                        rate_bps: None,
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
                        None,
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
                        Some(stderr_capture.clone()),
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
                        Ok(status) => (
                            None,
                            format!(
                                "yt-dlp exited with {status}{}",
                                ytdlp_failure_detail(&stderr_capture)
                            ),
                        ),
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
                rate_bps: None,
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

    pub fn set_max_downloads(&mut self, max: usize) {
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
        let gate = self.last_ytdlp_launch.clone();
        let res_tx = self.playlist_tx.clone();
        let handle = tokio::spawn(async move {
            let permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let results = run_playlist_fetch(&url, auth_args, gate, cancel_rx, permit).await;
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
    /// so the returned CDN URL is not a stale, HTTP-403'd one. Callers that
    /// cannot afford to hold the manager lock across the run should use
    /// [`resolve_stream_ytdlp`] with [`YoutubeManager::yt_extras`] instead.
    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo, String> {
        let auth = self.ytdlp_auth_args();
        let direct = resolve_stream_ytdlp(&self.semaphore, &self.last_ytdlp_launch, &auth, url)
            .await?;
        Ok(StreamInfo {
            url: direct,
            title: url.to_string(),
            ext: "m4a".to_string(),
            duration: 0.0,
        })
    }

    /// Resolve any yt-dlp-supported URL into its rendered title and a direct
    /// audio URL in a single extractor pass — the yt-dlp provider family
    /// (SoundCloud, Bandcamp, Mixcloud, ...).
    pub async fn resolve_info(&mut self, url: &str) -> Result<(String, String), String> {
        let auth = self.ytdlp_auth_args();
        resolve_info_ytdlp(&self.semaphore, &self.last_ytdlp_launch, &auth, url).await
    }
}

/// One-shot yt-dlp extraction of `url`'s audio stream with extra flags,
/// returning trimmed stdout (a direct URL with `-g`, or requested `--print`
/// fields). Run through the shared launch gate so concurrent resolves,
/// downloads and searches never burst YouTube's rate limiter; a 429/"Too
/// Many Requests" reply is retried with exponential backoff. Failures surface
/// the last stderr line plus a targeted hint.
async fn resolve_ytdlp(
    semaphore: &Semaphore,
    gate: &Arc<Mutex<Instant>>,
    auth: &[std::ffi::OsString],
    url: &str,
    extra: &[&str],
) -> Result<String, String> {
    let permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("semaphore: {e}"))?;

    let mut base_args: Vec<std::ffi::OsString> = vec![
        "--no-playlist".into(),
        "-f".into(),
        "bestaudio[ext=m4a]/bestaudio".into(),
    ];
    base_args.extend(extra.iter().map(|s| std::ffi::OsString::from(*s)));
    base_args.extend(auth.iter().cloned());

    let mut last_detail = String::from("unknown error");
    for attempt in 0..=YTDLP_RATE_LIMIT_RETRIES {
        gate_ytdlp_launch(gate).await;
        let mut args = base_args.clone();
        args.push(url.to_string().into());
        let output = timeout(SEARCH_TIMEOUT, Command::new("yt-dlp").args(&args).output())
            .await
            .map_err(|_| "resolve timeout".to_string())?
            .map_err(|e| format!("yt-dlp: {e}"))?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        last_detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error")
            .to_string();
        if is_rate_limited(&stderr) && attempt < YTDLP_RATE_LIMIT_RETRIES {
            let backoff = YTDLP_RATE_LIMIT_BACKOFF * (1u32 << attempt) as u32;
            debug!("yt-dlp rate limited; retrying in {}s", backoff.as_secs());
            tokio::time::sleep(backoff).await;
            continue;
        }
        break;
    }
    drop(permit);
    let hint = if last_detail.contains("403") || last_detail.contains("Forbidden") {
        " (cookies unavailable: set a logged-in cookies.txt in Settings → YouTube)"
    } else if last_detail.contains("429") || last_detail.contains("Too Many Requests") {
        " (YouTube rate limit: wait a moment, then retry)"
    } else {
        ""
    };
    Err(format!("yt-dlp resolve failed: {last_detail}{hint}"))
}

/// True when a yt-dlp stderr dump indicates the rate limiter fired.
fn is_rate_limited(stderr: &str) -> bool {
    stderr.contains("429") || stderr.to_ascii_lowercase().contains("too many requests")
}

/// Hold the shared launch gate until at least [`YTDLP_MIN_LAUNCH_GAP`] has
/// elapsed since the previous yt-dlp subprocess start, then stamp the new
/// start time. Because the gate stays held across the wait, callers are
/// served one at a time: launches land exactly `MIN_LAUNCH_GAP` apart even
/// when resolve/download/search/playlist tasks fire in the same instant.
async fn gate_ytdlp_launch(gate: &Arc<Mutex<Instant>>) {
    let mut guard = gate.lock().await;
    let since = Instant::now().saturating_duration_since(*guard);
    if since < YTDLP_MIN_LAUNCH_GAP {
        tokio::time::sleep(YTDLP_MIN_LAUNCH_GAP - since).await;
    }
    *guard = Instant::now();
}

/// Lock-free yt-dlp `-g` resolve of a YouTube watch URL into a direct audio
/// URL. Callers (the daemon's playback path) clone the shared semaphore and
/// launch gate from [`YoutubeManager::yt_extras`] and pass them in instead of
/// holding the `YoutubeManager` mutex across the yt-dlp run.
pub async fn resolve_stream_ytdlp(
    semaphore: &Semaphore,
    gate: &Arc<Mutex<Instant>>,
    auth: &[std::ffi::OsString],
    url: &str,
) -> Result<String, String> {
    let direct = resolve_ytdlp(semaphore, gate, auth, url, &["-g"]).await?;
    if direct.is_empty() {
        return Err("empty stream URL".to_string());
    }
    Ok(direct)
}

/// Lock-free resolve of any yt-dlp-supported URL into its rendered title and
/// a direct audio URL via `--print` (SoundCloud, Bandcamp, Mixcloud, ...).
pub async fn resolve_info_ytdlp(
    semaphore: &Semaphore,
    gate: &Arc<Mutex<Instant>>,
    auth: &[std::ffi::OsString],
    url: &str,
) -> Result<(String, String), String> {
    let out = resolve_ytdlp(
        semaphore,
        gate,
        auth,
        url,
        &["--print", "%(title)s", "--print", "%(url)s"],
    )
    .await?;
    let mut lines = out.lines();
    let title = lines
        .next()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or(url)
        .to_string();
    let direct = lines.next().unwrap_or("").trim().to_string();
    if direct.is_empty() {
        return Err("empty stream URL".to_string());
    }
    Ok((title, direct))
}

/// Download a YouTube URL into `dest_dir` under `prefix.<ext>` using yt-dlp,
/// so authenticated/PO-token extraction succeeds instead of answering HTTP 403.
/// Returns the path of the produced audio file. Joins the shared yt-dlp launch
/// queue (permit + spacing gate) and retries rate-limited runs with backoff so
/// the Spotify fallback cache cannot burst the extractor either.
pub(crate) async fn download_into(
    url: &str,
    dest_dir: &Path,
    prefix: &str,
    auth: &[std::ffi::OsString],
    semaphore: &Semaphore,
    gate: &Arc<Mutex<Instant>>,
) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("create download dir: {e}"))?;
    let template = dest_dir
        .join(format!("{prefix}.%(ext)s"))
        .to_string_lossy()
        .into_owned();

    let mut base_args: Vec<std::ffi::OsString> = vec![
        "-f".into(),
        "bestaudio[ext=m4a]/bestaudio".into(),
        "--no-playlist".into(),
        "-o".into(),
        template.into(),
    ];
    base_args.extend(auth.iter().cloned());

    let permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("semaphore: {e}"))?;

    let mut last_detail = String::from("unknown error");
    for attempt in 0..=YTDLP_RATE_LIMIT_RETRIES {
        gate_ytdlp_launch(gate).await;
        let mut args = base_args.clone();
        args.push(url.to_string().into());
        let output = timeout(
            Duration::from_secs(180),
            Command::new("yt-dlp").args(&args).output(),
        )
        .await
        .map_err(|_| "download timed out".to_string())?
        .map_err(|e| format!("yt-dlp: {e}"))?;

        if output.status.success() {
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
            if let Some(path) = produced {
                return Ok(path);
            }
            return Err("yt-dlp produced no file".to_string());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        last_detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error")
            .to_string();
        if is_rate_limited(&stderr) && attempt < YTDLP_RATE_LIMIT_RETRIES {
            let backoff = YTDLP_RATE_LIMIT_BACKOFF * (1u32 << attempt) as u32;
            debug!("yt-dlp download rate limited; retrying in {}s", backoff.as_secs());
            tokio::time::sleep(backoff).await;
            continue;
        }
        break;
    }
    drop(permit);
    let hint = if last_detail.contains("403") || last_detail.contains("Forbidden") {
        " (cookies unavailable: set a logged-in cookies.txt in Settings → YouTube)"
    } else if last_detail.contains("429") || last_detail.contains("Too Many Requests") {
        " (YouTube rate limit: wait a moment, then retry)"
    } else {
        ""
    };
    Err(format!("yt-dlp download failed: {last_detail}{hint}"))
}

/// Parse `[download] 42.3% of ...` progress lines emitted by yt-dlp with
/// `--newline` and publish them on the download progress channel. When
/// `capture` is set (stderr pipe), the raw text is also retained (bounded to
/// the last few dozen lines) so a failed run can report the real reason.
async fn spawn_progress_reader<R>(
    reader: R,
    progress_tx: mpsc::UnboundedSender<DownloadProgress>,
    download_id: u64,
    url: String,
    title: String,
    capture: Option<Arc<std::sync::Mutex<String>>>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(fields) = parse_dl_progress(&line) {
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
                rate_bps: fields.rate_bps,
                eta_secs: fields.eta_secs,
            });
        } else if let Some(cap) = &capture {
            let mut guard = cap.lock().unwrap();
            guard.push_str(&line);
            guard.push('\n');
            let mut kept: Vec<&str> = guard.lines().collect();
            if kept.len() > 40 {
                kept.drain(..kept.len() - 40);
            }
            *guard = format!("{}\n", kept.join("\n"));
        }
    }
}

/// Last non-empty stderr line captured from a failed yt-dlp run, with a
/// rate-limit/cookie hint appended when the text matches those failure modes.
fn ytdlp_failure_detail(capture: &std::sync::Mutex<String>) -> String {
    let stderr = { capture.lock().unwrap().clone() };
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if last.is_empty() {
        return String::new();
    }
    let hint = if last.contains("403") || last.contains("Forbidden") {
        " (cookies unavailable: set a logged-in cookies.txt in Settings → YouTube)"
    } else if last.contains("429") || last.contains("Too Many Requests") {
        " (YouTube rate limit: wait a moment, then retry)"
    } else {
        ""
    };
    format!(": {last}{hint}")
}

struct YtDlpLine {
    percent: f64,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    rate_bps: Option<f64>,
    eta_secs: Option<u64>,
}

/// Parse a yt-dlp progress line like
/// `[download]  42.3% of 3.86MiB at 1.10MiB/s ETA 00:00` and return the fields
/// we track. Sizes and rates use the exact units yt-dlp prints (`KiB/MiB/GiB`,
/// occasionally with a `~` for estimated totals), so they convert cleanly to
/// bytes.
fn parse_dl_progress(line: &str) -> Option<YtDlpLine> {
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
        rate_bps: extract_rate(line),
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
    auth_args: Vec<std::ffi::OsString>,
    gate: Arc<Mutex<Instant>>,
    cancel_rx: oneshot::Receiver<()>,
    _permit: OwnedSemaphorePermit,
) -> Vec<YTSearchResult> {
    // Provider source selector (`scsearch:`/`bilisearch:`/`mcsearch:` and any
    // `GTM_YT_HOSTS` additions) mirrors the yt-dlp extractor prefixes cliamp
    // exposes for its Youtube-family providers. The prefix is stripped here
    // and fed back to yt-dlp's search extractor with a 10-hit limit.
    if let Some((rest, host)) = match_yt_host(query, &yt_hosts()) {
        let search_arg = format!("{}:{rest}", host.extractor);
        return run_ytdlp_search(&search_arg, auth_args, gate, cancel_rx).await;
    }

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

/// Run a yt-dlp search-extractor query (`scsearch10:...`, `bilisearch10:...`,
/// `mcsearch10:...`, `ytsearch10:...`) and parse the JSON-lines output into
/// [`YTSearchResult`]s, mirroring the flat-playlist fetch used for URLs.
async fn run_ytdlp_search(
    search_arg: &str,
    auth_args: Vec<std::ffi::OsString>,
    gate: Arc<Mutex<Instant>>,
    cancel_rx: oneshot::Receiver<()>,
) -> Vec<YTSearchResult> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--flat-playlist".into(),
        "--dump-json".into(),
        "--no-warnings".into(),
        "--no-playlist".into(),
    ];
    args.extend(auth_args);
    args.push(search_arg.to_string().into());

    let fetch = async {
        gate_ytdlp_launch(&gate).await;
        let output =
            match timeout(SEARCH_TIMEOUT, Command::new("yt-dlp").args(&args).output()).await {
                Ok(res) => res.map_err(|e| format!("yt-dlp: {e}"))?,
                Err(_) => return Err("search timeout".to_string()),
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
            Err(format!("yt-dlp search: {detail}"))
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

/// Collect the entries of a YouTube playlist/URL via `yt-dlp --flat-playlist
/// --dump-json`, mapping each JSON-lines record into a [`YTSearchResult`]
/// until the caller cancels via `cancel_rx` or [`SEARCH_TIMEOUT`] elapses.
async fn run_playlist_fetch(
    url: &str,
    auth_args: Vec<std::ffi::OsString>,
    gate: Arc<Mutex<Instant>>,
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
        gate_ytdlp_launch(&gate).await;
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
    // Prefer the provider's own webpage URL (SoundCloud/Bilibili/Mixcloud
    // entries carry one); fall back to reconstructing a YouTube watch URL.
    let url = ["webpage_url", "url"]
        .iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .filter(|u| u.starts_with("http"))
        .map(|u| u.to_string())
        .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
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
    fn prog_fields() {
        let line = "[download]  42.3% of 3.86MiB at 1.10MiB/s ETA 00:07";
        let fields = parse_dl_progress(line).expect("progress line parses");
        assert!((fields.percent - 42.3).abs() < 1e-9);
        assert_eq!(fields.total_bytes, Some((3.86 * 1024.0 * 1024.0) as u64));
        assert!((fields.rate_bps.unwrap() - 1.10 * 1024.0 * 1024.0).abs() < 1.0);
        assert_eq!(fields.eta_secs, Some(7));
        assert_eq!(
            fields.downloaded_bytes,
            Some((0.423 * fields.total_bytes.unwrap() as f64) as u64)
        );
    }

    #[test]
    fn prog_eta_estimate() {
        let line = "[download]   5.0% of ~2.36GiB at 90.5KiB/s ETA 07:10";
        let fields = parse_dl_progress(line).expect("estimated total parses");
        assert!((fields.percent - 5.0).abs() < 1e-9);
        let gi = 1024.0 * 1024.0 * 1024.0;
        assert_eq!(fields.total_bytes, Some((2.36 * gi) as u64));
        assert!((fields.rate_bps.unwrap() - 90.5 * 1024.0).abs() < 1.0);
        assert_eq!(fields.eta_secs, Some(7 * 60 + 10));
    }

    #[test]
    fn prog_skips_junk() {
        assert!(parse_dl_progress("[download] Destination: /tmp/x.m4a").is_none());
        assert!(parse_dl_progress("[ExtractAudio] Destination: /tmp/x.m4a").is_none());
    }
}
