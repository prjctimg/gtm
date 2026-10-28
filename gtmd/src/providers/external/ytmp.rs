// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// yt-dlp search/extract helpers shared by yt-dlp-backed providers

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtm::providers::external::ProviderTrack;
use serde_json::Value;
use tokio::time::timeout;

use crate::config::ProviderConfig;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const YTDLP_MIN_LAUNCH_GAP: Duration = Duration::from_millis(1500);

static YTDLP_GATE: std::sync::OnceLock<Arc<tokio::sync::Mutex<Instant>>> =
    std::sync::OnceLock::new();

fn gate_arc() -> &'static Arc<tokio::sync::Mutex<Instant>> {
    YTDLP_GATE.get_or_init(|| {
        Arc::new(tokio::sync::Mutex::new(
            Instant::now() - YTDLP_MIN_LAUNCH_GAP,
        ))
    })
}

#[derive(Clone, Debug, Default)]
pub struct YtAuth {
    pub cookies_from: Option<String>,
    pub cookies_file: Option<PathBuf>,
    pub js_runtime: Option<String>,
}

impl YtAuth {
    pub fn from_config(cfg: &ProviderConfig, id: &str) -> Self {
        Self {
            cookies_from: cfg.string(id, "cookies_from"),
            cookies_file: cfg.string(id, "cookies").map(PathBuf::from),
            js_runtime: cfg.string(id, "js_runtime"),
        }
    }

    pub fn args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        if let Some(s) = &self.cookies_from {
            if !s.is_empty() {
                args.push("--cookies-from-browser".into());
                args.push(s.clone().into());
            }
        } else if let Some(p) = &self.cookies_file {
            args.push("--cookies".into());
            args.push(p.as_os_str().into());
        }
        if let Some(r) = &self.js_runtime
            && !r.is_empty()
        {
            args.push("--js-runtime".into());
            args.push(r.clone().into());
        }
        args
    }
}

async fn gate() {
    let g = gate_arc();
    let mut guard = g.lock().await;
    let since = guard.elapsed();
    if since < YTDLP_MIN_LAUNCH_GAP {
        tokio::time::sleep(YTDLP_MIN_LAUNCH_GAP - since).await;
    }
    *guard = Instant::now();
}

fn parse_track(v: &Value, provider_id: &str) -> Option<ProviderTrack> {
    let title = v.get("title").and_then(|t| t.as_str())?.to_string();
    if title.is_empty() {
        return None;
    }
    let id = ["webpage_url", "original_url", "url", "id"]
        .iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .filter(|u| u.starts_with("http"))?
        .to_string();
    let artist = ["channel", "uploader", "artist", "creator", "uploader_id"]
        .iter()
        .find_map(|k| v.get(k).and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    let duration = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
    let cover_url = v
        .get("thumbnails")
        .and_then(|t| t.as_array())
        .and_then(|arr| {
            arr.iter()
                .find_map(|x| x.get("url").and_then(|u| u.as_str()))
                .or_else(|| {
                    arr.first()
                        .and_then(|x| x.get("url").and_then(|u| u.as_str()))
                })
        })
        .map(|s| s.to_string());
    Some(ProviderTrack {
        provider: provider_id.to_string(),
        id,
        title,
        artist,
        album: String::new(),
        duration_secs: duration,
        cover_url,
        playlist_id: None,
    })
}

async fn run_ytdlp(args: Vec<OsString>) -> Result<std::process::Output, String> {
    gate().await;
    tokio::task::spawn_blocking(move || Command::new("yt-dlp").args(&args).output())
        .await
        .map_err(|e| format!("yt-dlp: {e}"))?
        .map_err(|e| format!("yt-dlp: {e}"))
}

pub async fn search(
    auth: &YtAuth,
    provider_id: &str,
    search_arg: &str,
    limit: usize,
) -> Result<Vec<ProviderTrack>, String> {
    let mut args: Vec<OsString> = vec![
        "--flat-playlist".into(),
        "--dump-json".into(),
        "--no-warnings".into(),
        "--no-playlist".into(),
        "--playlist-end".into(),
        format!("{}", limit.clamp(1, 50)).into(),
    ];
    args.extend(auth.args());
    args.push(search_arg.to_string().into());

    let output = timeout(SEARCH_TIMEOUT, run_ytdlp(args))
        .await
        .map_err(|_| "yt-dlp search timeout".to_string())??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(format!("yt-dlp: {detail}"));
    }
    let mut tracks = Vec::new();
    for line in output.stdout.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(t) = parse_track(&v, provider_id)
        {
            let url = t.id.as_str();
            if provider_id == "ytmusic" && url.contains("/browse/") && !url.contains("watch?v=") {
                continue;
            }
            tracks.push(t);
            if tracks.len() >= limit.clamp(1, 50) {
                break;
            }
        }
    }
    Ok(tracks)
}

pub async fn flat_playlist(
    auth: &YtAuth,
    provider_id: &str,
    url: &str,
    limit: usize,
) -> Result<Vec<ProviderTrack>, String> {
    let mut args: Vec<OsString> = vec![
        "--flat-playlist".into(),
        "--dump-json".into(),
        "--no-warnings".into(),
        "--playlist-end".into(),
        format!("{}", limit.clamp(1, 200)).into(),
    ];
    args.extend(auth.args());
    args.push(url.to_string().into());

    let output = timeout(SEARCH_TIMEOUT, run_ytdlp(args))
        .await
        .map_err(|_| "yt-dlp timeout".to_string())??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(format!("yt-dlp: {detail}"));
    }
    let mut tracks = Vec::new();
    for line in output.stdout.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(t) = parse_track(&v, provider_id)
        {
            tracks.push(t);
            if tracks.len() >= limit.clamp(1, 200) {
                break;
            }
        }
    }
    Ok(tracks)
}

pub async fn direct_url(auth: &YtAuth, url: &str) -> Result<String, String> {
    let mut args: Vec<OsString> = vec![
        "-g".into(),
        "-f".into(),
        "bestaudio/best".into(),
        "--no-playlist".into(),
    ];
    args.extend(auth.args());
    args.push(url.to_string().into());

    let output = timeout(Duration::from_secs(60), run_ytdlp(args))
        .await
        .map_err(|_| "yt-dlp resolve timeout".to_string())??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(format!("yt-dlp: {detail}"));
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let line = s
        .lines()
        .find(|l| l.starts_with("http"))
        .or_else(|| s.lines().next())
        .unwrap_or("")
        .trim()
        .to_string();
    if line.is_empty() {
        return Err("yt-dlp: empty stream url".to_string());
    }
    Ok(line)
}
