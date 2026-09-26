use super::*;

pub(crate) const RESTART_THRESHOLD_SECS: f64 = 3.0;

/// Which remote provider a synthetic queue path belongs to. The daemon
/// rebuilds the underlying stream URL from the manager owning the provider,
/// so the TUI never needs a fetch library. Only these synthetic schemes are
/// treated as remote; everything else goes down the local-file path.
pub(crate) enum RemoteKind {
    Podcast {
        feed_id: String,
        episode_index: usize,
    },
    Radio {
        station_id: String,
        station_name: String,
    },
    Stream {
        url: String,
    },
    /// yt-dlp extractor URLs (SoundCloud, Bandcamp, Mixcloud, ...): the URL
    /// resolves to a direct CDN audio URL through the yt-dlp subprocess.
    YtDlp {
        url: String,
    },
}

pub(crate) fn parse_remote_path(path: &str) -> Option<RemoteKind> {
    if let Some(rest) = path.strip_prefix("podcast://") {
        let (feed_id, index) = rest.split_once('/').unwrap_or((rest, "0"));
        let episode_index = index.parse::<usize>().unwrap_or(0);
        return Some(RemoteKind::Podcast {
            feed_id: feed_id.to_string(),
            episode_index,
        });
    }
    if let Some(rest) = path.strip_prefix("radio://") {
        let (station_id, station_name) = match rest.split_once('/') {
            Some((id, name)) => (id.to_string(), name.to_string()),
            None => (rest.to_string(), String::new()),
        };
        return Some(RemoteKind::Radio {
            station_id,
            station_name,
        });
    }
    if ytdlp_label(path).is_some() {
        return Some(RemoteKind::YtDlp { url: path.into() });
    }
    if path.starts_with("http://") || path.starts_with("https://") {
        return Some(RemoteKind::Stream { url: path.into() });
    }
    None
}

/// Result of resolving a provider remote path into a playable transport.
pub(crate) struct RemoteResolved {
    pub(crate) url: String,
    /// Whether the transport is live (non-seekable) radio/stream.
    pub(crate) live: bool,
}

/// Resolve a synthetic remote path into its playable HTTP stream URL, plus
/// whether it is a live (non-seekable) transport. Backed by the owning
/// provider manager so the TUI never needs a fetch library of its own.
pub(crate) async fn resolve_remote(inner: &DaemonInner, path: &str) -> Result<RemoteResolved, CoreError> {
    let kind = parse_remote_path(path)
        .ok_or_else(|| CoreError::Daemon(format!("{path} is not a remote provider path")))?;
    let url = match &kind {
        RemoteKind::Podcast {
            feed_id,
            episode_index,
        } => {
            let mut podcast = inner.podcast.lock().await;
            match podcast.episode_at(feed_id, *episode_index) {
                Some(ep) => ep.url.clone(),
                None => {
                    // Daemon restarted since the feed was browsed; refresh the
                    // feed once so replay/next still resolves.
                    podcast
                        .refresh_feed(feed_id)
                        .await
                        .map_err(|e| CoreError::Daemon(format!("podcast feed: {e}")))?;
                    podcast
                        .episode_at(feed_id, *episode_index)
                        .ok_or_else(|| CoreError::Daemon("podcast episode missing".into()))?
                        .url
                        .clone()
                }
            }
        }
        RemoteKind::Radio { station_id, .. } => {
            if let Some(index) = gtm::shared::custom::parse_custom_id(station_id) {
                gtm::shared::custom::station_by_index(index)
                    .map_err(CoreError::Daemon)?
                    .ok_or_else(|| CoreError::Daemon(format!("no custom station {index}")))?
                    .url
            } else {
                let radio = inner.radio.lock().await;
                radio
                    .by_uuid(station_id)
                    .await
                    .map_err(|e| CoreError::Daemon(format!("radio lookup: {e}")))?
                    .url_resolved
            }
        }
        RemoteKind::Stream { url } => url.clone(),
        #[cfg(feature = "youtube")]
        RemoteKind::YtDlp { url } => {
            // Resolve via yt-dlp without holding the `YoutubeManager` mutex
            // (extraction can take up to SEARCH_TIMEOUT, more under
            // rate-limit backoff; holding the lock would stall `YtDownload`
            // until the client's IPC timeout). The shared semaphore + launch
            // gate still serialize the run against every other yt-dlp site.
            let (auth, sem, gate) = {
                let yt = inner.youtube.lock().await;
                yt.yt_extras()
            };
            let (_, direct) = crate::youtube::resolve_info_ytdlp(&sem, &gate, &auth, url)
                .await
                .map_err(CoreError::Daemon)?;
            direct
        }
        #[cfg(not(feature = "youtube"))]
        RemoteKind::YtDlp { .. } => {
            return Err(CoreError::Daemon(
                "youtube support is disabled in this build".into(),
            ));
        }
    };
    let live = matches!(kind, RemoteKind::Radio { .. } | RemoteKind::Stream { .. });
    Ok(RemoteResolved { url, live })
}

/// Blocking HTTP open of a remote stream. Runs on a blocking thread; returns
/// a byte transport (`IcyReader` strips Shoutcast metadata blocks and publishes
/// `StreamTitle` into `title_slot`, `HttpReader` is the plain fallback). The
/// real decode happens later on the mixer's decode thread for live sources or
/// in [`decode_remote_reader`] for on-demand content.
pub(crate) fn open_remote_reader(
    url: &str,
    live: bool,
    title_slot: Option<remote::IcySlot>,
) -> AudioResult<Box<dyn std::io::Read + Send>> {
    // Live streams use a client with no total timeout: reqwest's `.timeout()`
    // covers the whole request including endless body streaming, so the
    // on-demand client would kill every station ~60s in.
    let mut req = if live {
        remote::live_client().get(url)
    } else {
        remote::client().get(url)
    };
    if live && title_slot.is_some() {
        req = req.header(remote::ICY_META_HEADER, "1");
    }
    let resp = req
        .send()
        .map_err(|e| AudioError::DecodeError(format!("stream {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AudioError::DecodeError(format!(
            "stream HTTP {}",
            resp.status()
        )));
    }
    let reader: Box<dyn std::io::Read + Send> = if let Some(slot) = title_slot.filter(|_| live) {
        if let Some(metaint) = resp
            .headers()
            .get(remote::ICY_META_INTERVAL)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
        {
            Box::new(remote::IcyReader::new(resp, metaint, slot))
        } else {
            Box::new(remote::HttpReader::from_response(resp))
        }
    } else {
        Box::new(remote::HttpReader::from_response(resp))
    };
    Ok(reader)
}

/// Blocking decode of an HTTP stream into a decodable source. Runs on a
/// blocking thread (network read during probe); returns the same source type
/// the local-file and Spotify paths feed into `load_active_decoded`.
pub(crate) fn decode_remote_reader(
    url: String,
    live: bool,
    start_pos: f64,
    title_slot: Option<remote::IcySlot>,
) -> AudioResult<Box<dyn rodio::Source<Item = f32> + Send>> {
    let reader = open_remote_reader(&url, live, title_slot.clone())?;
    // Live streams reconnect from scratch (same URL, fresh ICY negotiation);
    // seeks stay disabled for them at the symphonia level, but the re-opener
    // lets a dropped connection resume instead of ending the source.
    let reopen: Option<Box<dyn StreamingReopen>> = if live {
        Some(Box::new(remote::LiveReopen::new(url, title_slot)))
    } else {
        Some(Box::new(remote::HttpReopen::new(url)))
    };
    AudioMixer::decode_reader(reader, reopen, start_pos)
}

/// Derive a display title from a stream URL (host name or path segment).
pub(crate) fn title_from_url(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split(['/', '?']).next())
        .unwrap_or("Stream")
        .to_string()
}

/// Sniff whether `body` (fetched from `url`) is a playlist, and return the
/// raw path lines if so.
pub(crate) fn sniff_stream_playlist(url: &str, body: &str) -> Vec<String> {
    let trimmed = body.trim_start_matches('\u{feff}').trim();
    let ext_lower = std::path::Path::new(url)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let looks_pls = trimmed.starts_with("[playlist]");
    let looks_m3u = trimmed.starts_with("#EXTM3U") || trimmed.starts_with("#EXTINF");
    match ext_lower.as_deref() {
        Some("pls") => PlsFormat.parse_track_lines(body),
        Some("m3u") | Some("m3u8") => M3u8Format.parse_track_lines(body),
        _ if looks_pls => PlsFormat.parse_track_lines(body),
        _ if looks_m3u => M3u8Format.parse_track_lines(body),
        _ => Vec::new(),
    }
}

/// Resolve an M3U/PLS entry line against the playlist's base URL.
pub(crate) fn resolve_stream_entry(base: &str, entry: &str) -> Option<String> {
    let e = entry.trim();
    if e.is_empty() || e.starts_with('#') {
        return None;
    }
    if e.starts_with("http://") || e.starts_with("https://") {
        return Some(e.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    let parent = rest.rfind('/').map(|i| &rest[..i]).unwrap_or("");
    Some(format!("{scheme}://{parent}/{}", e.trim_start_matches('/')))
}
