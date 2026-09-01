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
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::debug;

use gtm_core::global::YTFilter;
use gtm_core::track::{StreamInfo, YTSearchResult};

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
            // The InnerTube client is built lazily on first async use (see
            // `ensure_client`) because `Innertube::with_options` is async and
            // `YoutubeManager::new()` must stay synchronous. The client shares
            // a single HTTP session + QuickJS decipher engine behind Arcs.
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
        }
    }

    /// Build (once) and hand back a cloneable InnerTube client. Errors are
    /// surfaced so callers can report a helpful message instead of silently
    /// producing empty results.
    async fn ensure_client(&mut self) -> Result<Innertube, String> {
        if let Some(c) = &self.client {
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
        Ok(client)
    }

    pub fn set_cookie_file(&mut self, path: Option<String>) {
        self.cookie_file = path.map(PathBuf::from);
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
    pub async fn start_search(&mut self, query: &str, filter: Option<YTFilter>) -> Result<(), String> {
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
    pub async fn download_to_path(url: &str, dest: &Path) -> Result<(), String> {
        let resp = reqwest::Client::builder()
            .build()
            .map_err(|e| format!("build http client: {e}"))?
            .get(url)
            .send()
            .await
            .map_err(|e| format!("download: {e}"))?;
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
        match self.cookie_file.as_ref() {
            Some(p) if p.is_file() => parse_cookie_file(p),
            _ => None,
        }
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
                    url: format!(
                        "https://www.youtube.com/playlist?list={}",
                        p.playlist_id
                    ),
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
        views: v.view_count.as_deref().and_then(parse_view_count).unwrap_or(0),
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

/// Parse a Netscape cookies.txt file into a HTTP `Cookie` header string.
fn parse_cookie_file(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut pairs: Vec<(String, String)> = Vec::new();
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
            pairs.push((name.to_string(), value.to_string()));
        }
    }
    if pairs.is_empty() {
        return None;
    }
    Some(
        pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}
