use super::*;
use gtm::shared::{SRC_CHART, SRC_LOCAL, SRC_PODCAST, SRC_PROVIDER, SRC_RADIO, SRC_SPOTIFY};

pub(crate) const RESTART_THRESHOLD_SECS: f64 = 3.0;

/// Which remote provider a synthetic queue path belongs to. The daemon
/// rebuilds the underlying stream URL from the manager owning the provider,
/// so the TUI never needs a fetch library. Only these synthetic schemes are
/// treated as remote; everything else goes down the local-file path.
#[derive(Debug)]
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
    /// A charts provider's stream: an `http(s)` URL a chart handed us, which
    /// fetches exactly like [`Stream`](Self::Stream) and lives at exactly the
    /// same place.
    ///
    /// Its own variant, rather than folded into `Stream`, for one reason: a
    /// chart row's playable URI is a bare `https://` link, and so is a station
    /// the user pasted by hand. Both resolve identically, so nothing else
    /// distinguishes them — but the source marker on the wire does, and a
    /// client showing "radio" over an Apple chart would be telling the user
    /// something false. The prefix is what carries the distinction, exactly as
    /// `radio://` and `podcast://` carry theirs.
    Chart {
        url: String,
    },
    /// yt-dlp extractor URLs (SoundCloud, Bandcamp, Mixcloud, ...): the URL
    /// resolves to a direct CDN audio URL through the yt-dlp subprocess.
    YtDlp {
        url: String,
    },
    /// A catalogue provider track: `provider://<provider>/<track id>`.
    ///
    /// The id is opaque to the daemon; the registry asks the owning client to
    /// resolve it. Keeping the provider and id in the path is what lets a row
    /// be replayed or auto-advanced hours later without the client being
    /// connected, and re-resolves signed URLs that have since expired.
    Provider {
        provider: String,
        id: String,
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
    // Checked before the `http(s)` arms below, and the whole URL kept rather
    // than just the host: a chart hands out signed CDN links whose query string
    // is part of the address.
    if let Some(rest) = path.strip_prefix("chart://") {
        return Some(RemoteKind::Chart { url: rest.into() });
    }
    if let Some(rest) = path.strip_prefix("provider://") {
        let (provider, id) = rest.split_once('/')?;
        if provider.is_empty() || id.is_empty() {
            return None;
        }
        return Some(RemoteKind::Provider {
            provider: provider.into(),
            id: id.into(),
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

/// The host of a bare stream URL, as a display name.
///
/// Strips the scheme, any credentials, the port and a leading `www.`, which
/// leaves the part a listener would actually recognise. Falls back to the whole
/// URL when there is no parseable host, so the label is never empty.
pub(crate) fn stream_label(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = rest.split_once(':').map_or(rest, |(h, _)| h);
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() { url } else { host }
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
pub(crate) async fn resolve_remote(
    inner: &DaemonInner,
    path: &str,
) -> Result<RemoteResolved, CoreError> {
    let kind = parse_remote_path(path)
        .ok_or_else(|| CoreError::Daemon(format!("{path} is not a remote provider path")))?;
    // Extractors need a human title for their search fallback, and a provider
    // may have queued the row under a name the id does not carry. The queue is
    // the authority when it has the row; an empty title is still valid for
    // providers that resolve purely by id.
    let queued_title = {
        let state = inner.state.read().await;
        state
            .queue
            .iter()
            .find(|t| t.path == path)
            .map(|t| t.title.clone())
            .unwrap_or_default()
    };
    let (url, live) = match &kind {
        RemoteKind::Podcast {
            feed_id,
            episode_index,
        } => {
            let mut podcast = inner.podcast.lock().await;
            let url = match podcast.episode_at(feed_id, *episode_index) {
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
            };
            (url, false)
        }
        RemoteKind::Radio { station_id, .. } => {
            let url = if let Some(index) = gtm::shared::custom::parse_custom_id(station_id) {
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
            };
            (url, true)
        }
        RemoteKind::Stream { url } => (url.clone(), true),
        RemoteKind::Chart { url } => (url.clone(), true),
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
            (direct, false)
        }
        #[cfg(not(feature = "youtube"))]
        RemoteKind::YtDlp { .. } => {
            return Err(CoreError::Daemon(
                "youtube support is disabled in this build".into(),
            ));
        }
        RemoteKind::Provider { provider, id } => {
            let stream = inner
                .providers
                .resolve(provider, id, &queued_title)
                .await
                .map_err(|e| CoreError::Daemon(format!("{}: {e}", provider)))?;
            (stream.url, stream.live)
        }
    };
    Ok(RemoteResolved { url, live })
}

/// The `SRC_*` value for a remote path.
///
/// On the path rather than on the request, because the path is what survives
/// every hop that can start playback: a queued row, a resumed position and a
/// crossfaded promotion all re-derive it from the same string, so none of them
/// can disagree about where a track came from.
impl RemoteKind {
    pub(crate) fn source(&self) -> &'static str {
        match self {
            RemoteKind::Podcast { .. } => SRC_PODCAST,
            RemoteKind::Radio { .. } => SRC_RADIO,
            RemoteKind::Chart { .. } => SRC_CHART,
            // A bare `https://` is a live stream by the only evidence there is:
            // it has no length, and that is what the daemon already treats it
            // as when it subscribes for ICY titles.
            RemoteKind::Stream { .. } => SRC_RADIO,
            // yt-dlp resolves to a direct CDN audio URL that the mixer decodes
            // as a file. The bytes are remote; the playback is local, and
            // `local` is the marker that means "this ends".
            RemoteKind::YtDlp { .. } => SRC_LOCAL,
            // A catalogue track has a duration and end, but the bytes are
            // remote and the row may need re-resolving. `provider` says both.
            RemoteKind::Provider { .. } => SRC_PROVIDER,
        }
    }
}

/// The queue path for a charts provider's playable URL.
///
/// Written next to [`parse_remote_path`] rather than in the chart providers,
/// because the prefix is a two-sided convention: the provider emits it and this
/// module reads it back. Two definitions of the same string in two files is
/// how a chart row ends up silently reported as a radio stream.
///
/// An empty URL stays empty. `chart://` with nothing behind it would parse,
/// resolve, and then fail on a fetch, which is a worse error to read than the
/// blank row it came from.
pub(crate) fn chart_uri(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    format!("chart://{url}")
}

/// The `SRC_*` value for any track path, local paths included.
///
/// The one entry point, so nothing has to remember that `spotify:` is handled
/// before `parse_remote_path` and that a path matching neither is a file.
pub(crate) fn source_for_path(path: &str) -> &'static str {
    if path.starts_with("spotify:") {
        return SRC_SPOTIFY;
    }
    match parse_remote_path(path) {
        Some(kind) => kind.source(),
        None => SRC_LOCAL,
    }
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

/// Blocking open of a remote stream's byte transport plus the re-opener that
/// gets back to the start of it.
///
/// Runs on a blocking thread; returns a byte transport (`IcyReader` strips
/// Shoutcast metadata blocks and publishes `StreamTitle` into `title_slot`,
/// `HttpReader` is the plain fallback) and, for live streams, a re-opener that
/// reconnects from scratch (same URL, fresh ICY negotiation) so a dropped
/// connection resumes and a seek restarts the transport instead of being
/// dropped.
pub(crate) fn open_remote_stream(
    url: &str,
    live: bool,
    title_slot: Option<remote::IcySlot>,
) -> AudioResult<(Box<dyn std::io::Read + Send>, Box<dyn StreamingReopen>)> {
    let reader = open_remote_reader(url, live, title_slot.clone())?;
    let reopen: Box<dyn StreamingReopen> = if live {
        Box::new(remote::LiveReopen::new(url, title_slot))
    } else {
        Box::new(remote::HttpReopen::new(url))
    };
    Ok((reader, reopen))
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
    let (reader, reopen) = open_remote_stream(&url, live, title_slot)?;
    AudioMixer::decode_reader(reader, Some(reopen), start_pos)
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
