use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::{oneshot, Semaphore};
use tokio::time::timeout;
use tracing::{debug, warn};

use gtm_core::state::YTFilter;
use gtm_core::track::{StreamInfo, YTSearchResult};

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENT: usize = 2;
const SEARCH_COUNT: usize = 10;

pub struct YoutubeManager {
    task: Option<oneshot::Sender<()>>,
    results: Vec<YTSearchResult>,
    semaphore: Semaphore,
}

impl YoutubeManager {
    pub fn new() -> Self {
        Self {
            task: None,
            results: Vec::new(),
            semaphore: Semaphore::new(MAX_CONCURRENT),
        }
    }

    pub async fn search(&mut self, query: &str, _filter: Option<YTFilter>) -> Result<(), String> {
        self.cancel().await;
        self.results.clear();

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("semaphore: {e}"))?;

        let search_arg = if query.starts_with("http://") || query.starts_with("https://") {
            query.to_string()
        } else {
            format!("ytsearch{}:{} official audio", SEARCH_COUNT, query)
        };

        let mut child = Command::new("yt-dlp")
            .arg(&search_arg)
            .arg("--dump-json")
            .arg("--flat-playlist")
            .arg("--no-warnings")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn yt-dlp: {e}"))?;

        let stdout = child.stdout.take().ok_or("no stdout")?;
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let (tx, mut rx) = oneshot::channel();
        self.task = Some(tx);

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
                _ = &mut rx => {
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
            b_prio.cmp(&a_prio).then(
                b.views.cmp(&a.views)
            )
        });
        self.results = results;
        Ok(())
    }

    pub async fn poll_results(&mut self) -> Result<Option<Vec<YTSearchResult>>, String> {
        if self.results.is_empty() {
            Ok(None)
        } else {
            Ok(Some(std::mem::take(&mut self.results)))
        }
    }

    pub async fn cancel(&mut self) {
        if let Some(tx) = self.task.take() {
            let _ = tx.send(());
        }
        self.results.clear();
    }

    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo, String> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| format!("semaphore: {e}"))?;

        let output = timeout(SEARCH_TIMEOUT, Command::new("yt-dlp")
            .arg("-g")
            .arg("-f")
            .arg("bestaudio[ext=m4a]/bestaudio")
            .arg(url)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output())
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
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("json: {e}"))?;
    let id = v.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let is_playlist = v.get("_type").and_then(|v| v.as_str()) == Some("playlist");
    Ok(YTSearchResult {
        id: id.clone(),
        title: v.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        url: if is_playlist {
            format!("https://www.youtube.com/playlist?list={id}")
        } else {
            format!("https://www.youtube.com/watch?v={id}")
        },
        channel: v.get("channel").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        duration: v.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
        views: v.get("view_count").and_then(|v| v.as_u64()).unwrap_or(0),
        thumbnail: v.get("thumbnail").and_then(|v| v.as_str()).map(|s| s.to_string()),
        is_playlist,
    })
}
