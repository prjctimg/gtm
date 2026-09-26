// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon event loop and IPC command handlers
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{error, info, warn};

use gtm::shared::paths::resolve_pid_file;

use crate::config::AudioBackendKind;
use base64::Engine;
#[cfg(feature = "pulseaudio")]
use gtm::audio::PulseAudioMixer;
use gtm::audio::symphonia::StreamingReopen;
use gtm::audio::{AudioError, AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
#[cfg(feature = "mpris")]
use gtm::mpris::{MprisHandle, start};
use gtm::shared::global::{
    DaemonState, EQ_PRESETS, EqPreset, LoudnessMode, PlaybackStatus, RepeatMode, ReverbConfig,
    SavedState, YTFilter,
};
use gtm::shared::ipc::{
    CacheKind, ComponentHealth, DaemonEvent, DaemonReq, DaemonRes, HealthReport, HealthStatus,
    LibraryAction, QueueAction, SyncKind, WireReq,
};
use gtm::shared::playlist::{M3u8Format, PlaylistFormat, PlsFormat};
use gtm::shared::secret::{
    LASTFM_API_KEY, LASTFM_API_SECRET, delete_secret, get_secret, set_secret,
};
use gtm::shared::spotify::SpotifyTrack;
use gtm::shared::track::{StreamInfo, TrackInfo};
use gtm::shared::url::{is_youtube, ytdlp_label};
use gtm::shared::wire;
use gtm::shared::{CoreError, MetadataPatch};
#[cfg(feature = "pulseaudio")]
use gtm::shared::{ensure_termux_pulse, is_termux};
use rspotify::AuthCodePkceSpotify;

use crate::charts::ChartsRegistry;
use crate::cleaner::{
    clean_filename_stem, clean_youtube_title, is_filename_like, sanitize_text, tags_need_enrichment,
};
use crate::config::DaemonConfig;
use crate::cover::{CoverCache, CoverProvider};
use crate::deezer::DeezerSearch;
use crate::deferred_mixer::DeferredMixer;
use crate::lastfm::LastfmManager;
use crate::library::{Library, extract_metadata};
use crate::lyrics::{LyricsManager, lrc_to_text, meta_from_filename};
use crate::network;
use crate::oauth::{OAUTH_TIMEOUT, OauthFlow, bind_callback};
use crate::podcast::PodcastManager;
use crate::queue;
use crate::radio::RadioBrowserManager;
use crate::remote;
use crate::spotify::{
    SpotifyManager, access_token, album_cover, album_tracks, artist_image, artist_top, image_at,
    search, web_playlist,
};
use crate::stream::StreamManager;
use crate::tags::{MetadataToWrite, write_tags};
#[cfg(feature = "youtube")]
use crate::youtube::{YoutubeManager, download_into};

type ClientId = u64;
type ReplyTx = mpsc::UnboundedSender<(u64, DaemonRes)>;

pub mod charts;
pub mod cover;
pub mod favourites;
pub mod health;
pub mod history;
pub mod lastfm;
pub mod library;
pub mod lyrics;
pub mod podcast;
pub mod queue_cmd;
pub mod radio;
pub mod scrobble;
pub mod search;
pub mod spotify;
pub mod stream;
pub mod yt;
pub mod ytfb;

#[cfg(test)]
mod tests;
pub(crate) use charts::*;
pub(crate) use cover::*;
pub(crate) use favourites::*;
pub(crate) use health::*;
pub(crate) use history::*;
pub(crate) use lastfm::*;
pub(crate) use library::*;
pub(crate) use lyrics::*;
pub(crate) use podcast::*;
pub(crate) use queue_cmd::*;
pub(crate) use radio::*;
pub(crate) use scrobble::*;
pub(crate) use search::*;
pub(crate) use spotify::*;
pub(crate) use stream::*;
pub(crate) use yt::*;
pub(crate) use ytfb::*;

struct Cmd;

impl Cmd {
    /// Scrobble the track that is about to be left. Uses the frame-derived
    /// listened-time tracker so seek-backwards never steals played credit, and
    /// skips live/infinite streams (duration 0) outright.
    async fn scrobble_track(inner: &DaemonInner, track: &TrackInfo, fallback_pos: f64) {
        if track.duration <= 0.0 {
            return;
        }
        let (enabled, min_secs, min_pct) = {
            let state = inner.state.read().await;
            (
                state.scrobble.enabled,
                state.scrobble.effective_play_secs(),
                state.scrobble.effective_play_pct(),
            )
        };
        if !enabled {
            return;
        }
        let played_secs = inner
            .scrobble
            .lock()
            .await
            .listened_for(&track.path, fallback_pos);
        let lastfm = inner.lastfm.lock().await;
        if lastfm.is_ready().await {
            let _ = tokio::time::timeout(
                Duration::from_secs(10),
                lastfm.scrobble(track, played_secs.max(0.0), min_secs, min_pct),
            )
            .await;
        }
    }

    pub async fn play(
        inner: &DaemonInner,
        path: &str,
        start_pos: f64,
        auto_advanced: bool,
    ) -> Result<DaemonRes, CoreError> {
        // Bump first so any in-flight auto-advance/crossfade task sees a
        // session change and backs out before it touches state.
        inner.play_session.fetch_add(1, Ordering::Release);
        if path.starts_with("spotify:") {
            return Cmd::play_stream(inner, path, start_pos, auto_advanced).await;
        }
        if parse_remote_path(path).is_some() {
            return Cmd::play_remote(inner, path, start_pos, auto_advanced).await;
        }
        // Local file: halt any active librespot stream so it stops decoding.
        inner.stream.lock().await.reset();
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
            // Ensure the mixer's speed matches the persisted state before loading
            // the next track. This guards against any drift between state and mixer.
            let speed = inner.state.read().await.audio.speed;
            mixer.set_speed(speed);
        }
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut state = inner.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        let path_owned = path.to_string();
        let path_for_blocking = path_owned.clone();
        let source =
            tokio::task::spawn_blocking(move || AudioMixer::decode_file(&path_for_blocking))
                .await
                .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
                .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;

        let dur = {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(source, start_pos)?;
            mixer.play()?;
            mixer.duration()
        };

        // Reuse metadata already held by the queue or the current track instead
        // of re-resolving from disk/library on every play. Stepping through a
        // queue (next/prev/auto-advance) regenerates identical metadata, and
        // the full resolution path (SQLite open, a potential full-library scan,
        // plus a whole-file hash and a second probe) is the dominant cost here.
        // Only fall back to a complete resolve when no matching entry exists.
        let queued = {
            let state = inner.state.read().await;
            state
                .queue
                .iter()
                .find(|t| t.path == path)
                .cloned()
                .or_else(|| {
                    state
                        .current_track
                        .as_ref()
                        .filter(|t| t.path == path)
                        .cloned()
                })
        };
        let track = match queued {
            Some(t) if t.duration > 0.0 || dur <= 0.0 => t,
            _ => Daemon::resolve_track_meta(inner, std::path::Path::new(&path_owned), dur).await,
        };

        // Scrobble previous track before switching. The state guard is taken
        // and dropped in a single scoped block so it is always released (a
        // second write() below would otherwise deadlock when there was no
        // previous track to scrobble).
        let (prev_track, prev_pos) = {
            let mut state = inner.state.write().await;
            let prev = state.current_track.take();
            let pos = state.time_pos.max(0.0);
            (prev, pos)
        };
        if let Some(prev_track) = prev_track {
            Cmd::scrobble_track(inner, &prev_track, prev_pos).await;
            let mut state = inner.state.write().await;
            state.current_track = Some(prev_track); // Restore for potential re-play
        }

        let mut state = inner.state.write().await;
        if let Some(pos) = state.queue.iter().position(|t| t.path == track.path)
            && pos > 0
        {
            state.queue.rotate_left(pos);
        }
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        // Drop the write guard before the Last.fm block below, which reads
        // the state again (a read while this write is alive would deadlock).
        drop(state);

        inner.scrobble.lock().await.start(&track.path, start_pos);

        // Broadcast the new track as soon as the mixer is playing, before the
        // Last.fm now-playing handshake below (which can hang for up to 10s),
        // so the TUI's Now Playing pane and cover art update immediately.
        Daemon::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced,
                time_pos: start_pos,
                duration: dur,
            },
        );

        // Update Last.fm now playing
        if inner.lastfm.lock().await.is_ready().await {
            let track_for_np = {
                let state = inner.state.read().await;
                if state.scrobble.enabled {
                    state.current_track.clone()
                } else {
                    None
                }
            };
            if let Some(ref track) = track_for_np {
                // Bound the now-playing handshake hard so a slow Last.fm
                // response never delays the play-command reply; the real
                // scrobble uses the background tracker.
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    inner.lastfm.lock().await.update_now_playing(track),
                )
                .await;
            }
        }
        Ok(DaemonRes::Ok)
    }

    /// Play a `spotify:track:<id>` URI through the librespot streaming
    /// bridge. Requires a linked Premium account; the queue entry (created
    /// at resolve time) already carries title/artist/album metadata.
    async fn play_stream(
        inner: &DaemonInner,
        uri_path: &str,
        start_pos: f64,
        auto_advanced: bool,
    ) -> Result<DaemonRes, CoreError> {
        inner.play_session.fetch_add(1, Ordering::Release);
        // Read every gate and the client under one lock, then refresh the token
        // off-lock so a stalled Spotify request cannot block every other
        // transport command.
        let (client, premium, relink) = {
            let spotify = inner.spotify.lock().await;
            (
                spotify.client(),
                spotify.is_premium(),
                spotify.needs_relink(),
            )
        };
        let Some(client) = client else {
            return Ok(DaemonRes::Error {
                message: "spotify not linked".into(),
            });
        };
        if !premium {
            return Ok(DaemonRes::Error {
                message: "spotify streaming requires a Premium account".into(),
            });
        }
        if relink {
            return Ok(DaemonRes::Error {
                message: "spotify token lacks the streaming scope; re-link the account".into(),
            });
        }
        let token = match access_token(&client).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(DaemonRes::Error {
                    message: format!("{e}; re-link the account"),
                });
            }
        };
        let duration_hint = {
            let state = inner.state.read().await;
            state
                .queue
                .iter()
                .find(|t| t.path == uri_path)
                .map(|t| t.duration)
                .filter(|d| *d > 0.0)
        };
        let config_dir = inner.config.config_dir.clone();

        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
        }
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut state = inner.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        // Scrobble whatever was playing before the stream is swapped out.
        let (prev_track, prev_pos) = {
            let mut state = inner.state.write().await;
            let prev = state.current_track.take();
            let pos = state.time_pos.max(0.0);
            (prev, pos)
        };
        if let Some(prev_track) = prev_track {
            Cmd::scrobble_track(inner, &prev_track, prev_pos).await;
            let mut state = inner.state.write().await;
            state.current_track = Some(prev_track);
        }

        let source = {
            let mut stream = inner.stream.lock().await;
            match stream
                .load(
                    uri_path,
                    (start_pos.max(0.0) * 1000.0) as u32,
                    duration_hint.unwrap_or(0.0),
                    &token,
                    &config_dir,
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    return Ok(DaemonRes::Error {
                        message: format!("spotify stream: {e}"),
                    });
                }
            }
        };
        let dur = {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(Box::new(source), start_pos)?;
            mixer.play()?;
            mixer.duration()
        };

        let mut state = inner.state.write().await;
        let mut track = state.queue.iter().find(|t| t.path == uri_path).cloned();
        let track = match track.as_mut() {
            Some(t) => {
                t.duration = dur;
                t.clone()
            }
            None => TrackInfo {
                path: uri_path.to_string(),
                title: "Spotify Track".to_string(),
                duration: dur,
                ..Default::default()
            },
        };
        if let Some(pos) = state.queue.iter().position(|t| t.path == uri_path)
            && pos > 0
        {
            state.queue.rotate_left(pos);
        }
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        drop(state);
        inner.scrobble.lock().await.start(&track.path, start_pos);
        Daemon::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced,
                time_pos: start_pos,
                duration: dur,
            },
        );
        Ok(DaemonRes::Ok)
    }

    /// Play a synthetic remote path (`podcast://`, `radio://`, `stream://`)
    /// by streaming the underlying HTTP URL through the native decoder. Reuses
    /// the same `load_active_decoded` pipeline as local files and Spotify, so
    /// EQ, speed, crossfade-standby and gapless handling all agree on the
    /// decoded sample stream.
    async fn play_remote(
        inner: &DaemonInner,
        path: &str,
        start_pos: f64,
        auto_advanced: bool,
    ) -> Result<DaemonRes, CoreError> {
        inner.play_session.fetch_add(1, Ordering::Release);
        let resolved = resolve_remote(inner, path).await?;
        let (url, live) = (resolved.url, resolved.live);
        let kind = parse_remote_path(path)
            .ok_or_else(|| CoreError::Daemon(format!("{path} is not a remote provider path")))?;

        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
        }
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut state = inner.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        // Scrobble whatever was playing before the remote stream starts.
        let (prev_track, prev_pos) = {
            let mut state = inner.state.write().await;
            let prev = state.current_track.take();
            let pos = state.time_pos.max(0.0);
            (prev, pos)
        };
        if let Some(prev_track) = prev_track {
            Cmd::scrobble_track(inner, &prev_track, prev_pos).await;
            let mut state = inner.state.write().await;
            state.current_track = Some(prev_track);
        }

        // Fetch + probe the provider's stream off the async thread. Live radio
        // does not reconnect for seeks; podcast streams do. Reset any stale
        // live title before (re)connecting.
        *inner.icy_title.lock().unwrap() = None;
        {
            let mut state = inner.state.write().await;
            state.radio_title = None;
        }
        let start = start_pos.max(0.0);
        let icy_slot = inner.icy_title.clone();
        let dur = {
            let decoded = tokio::task::spawn_blocking(move || {
                decode_remote_reader(url, live, start, Some(icy_slot))
            })
            .await
            .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
            .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(decoded, start_pos)?;
            mixer.play()?;
            mixer.duration()
        };

        // Favour the existing queue entry's metadata (built client-side from
        // provider data); fall back to a default shaped by the provider kind.
        let mut state = inner.state.write().await;
        let mut queue_meta = state.queue.iter().find(|t| t.path == path).cloned();
        let track = match queue_meta.as_mut() {
            Some(t) => {
                if dur > 0.0 {
                    t.duration = dur;
                }
                t.clone()
            }
            None => {
                let (title, artist, album) = match &kind {
                    RemoteKind::Podcast { .. } => ("Podcast Episode", "Podcast", "Podcast"),
                    RemoteKind::Radio {
                        station_name: name, ..
                    } => (name.as_str(), "Radio", "Radio"),
                    RemoteKind::Stream { .. } => ("Stream", "Internet Radio", "Stream"),
                    RemoteKind::YtDlp { url } => {
                        let label = ytdlp_label(url).unwrap_or("YouTube");
                        (label, label, label)
                    }
                };
                TrackInfo {
                    path: path.to_string(),
                    title: title.to_string(),
                    artist: artist.to_string(),
                    album: album.to_string(),
                    duration: if dur > 0.0 { dur } else { 0.0 },
                    ..Default::default()
                }
            }
        };
        if let Some(pos) = state.queue.iter().position(|t| t.path == path)
            && pos > 0
        {
            state.queue.rotate_left(pos);
        }
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        drop(state);

        inner.scrobble.lock().await.start(&track.path, start_pos);

        // Broadcast before the Last.fm handshake so Now Playing syncs fast.
        Daemon::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced,
                time_pos: start_pos,
                duration: dur,
            },
        );

        let lastfm = inner.lastfm.lock().await;
        if lastfm.is_ready().await {
            let track_for_np = inner.state.read().await.current_track.clone();
            if let Some(ref track) = track_for_np {
                // Bounded to keep the play-command reply fast; Last.fm is
                // best-effort and must not stall playback control.
                let _ =
                    tokio::time::timeout(Duration::from_secs(1), lastfm.update_now_playing(track))
                        .await;
            }
        }
        Ok(DaemonRes::Ok)
    }
}

impl Cmd {
    /// Play a raw HTTP(S) stream URL, transparently resolving M3U/PLS
    /// playlists fetched from the URL. Remaining playlist entries stay in the
    /// queue so `next` rotates through them.
    pub async fn play_url_stream(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        inner.play_session.fetch_add(1, Ordering::Release);
        // yt-dlp family URLs (SoundCloud, Bandcamp, Mixcloud, ...) resolve to
        // a direct CDN audio URL through the extractor; the queue entry keeps
        // the extracted title so the now-playing pane shows real names.
        if let Some(label) = ytdlp_label(url) {
            #[cfg(feature = "youtube")]
            {
                let (auth, sem, gate) = {
                    let yt = inner.youtube.lock().await;
                    yt.yt_extras()
                };
                let (title, direct) =
                    match crate::youtube::resolve_info_ytdlp(&sem, &gate, &auth, url).await {
                        Ok(v) => v,
                        Err(e) => return Ok(DaemonRes::Error { message: e }),
                    };
                {
                    let mut state = inner.state.write().await;
                    state.queue.push(TrackInfo {
                        path: direct.clone(),
                        title,
                        artist: label.to_string(),
                        album: label.to_string(),
                        ..Default::default()
                    });
                }
                return Cmd::play(inner, &direct, 0.0, false).await;
            }
            #[cfg(not(feature = "youtube"))]
            {
                return Ok(DaemonRes::Error {
                    message: format!("{label} support is disabled in this build"),
                });
            }
        }
        let url_owned = url.to_string();
        let fetched =
            tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), String> {
                let resp = remote::client()
                    .get(&url_owned)
                    .send()
                    .map_err(|e| format!("stream fetch: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("stream HTTP {}", resp.status()));
                }
                let body = resp.text().map_err(|e| format!("stream read: {e}"))?;
                let parsed = sniff_stream_playlist(&url_owned, &body);
                Ok((url_owned, parsed))
            })
            .await
            .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
            .map_err(CoreError::Daemon)?;

        let (base, mut entries) = fetched;
        if entries.is_empty() {
            entries.push(base.clone());
        } else {
            entries = entries
                .iter()
                .filter_map(|e| resolve_stream_entry(&base, e))
                .collect();
            if entries.is_empty() {
                entries.push(base);
            }
        }

        let first = entries.remove(0);
        let remainder: Vec<TrackInfo> = entries
            .iter()
            .map(|u| TrackInfo {
                path: u.clone(),
                title: title_from_url(u),
                artist: "Stream".into(),
                album: "Stream".into(),
                ..Default::default()
            })
            .collect();
        if !remainder.is_empty() {
            let mut state = inner.state.write().await;
            state.queue.extend(remainder);
            drop(state);
        }
        Cmd::play(inner, &first, 0.0, false).await
    }

    pub async fn play_pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let is_playing = inner.mixer.lock().await.is_playing();
        if is_playing {
            Cmd::pause(inner).await
        } else {
            let state = inner.state.read().await;
            let is_paused = state.status == PlaybackStatus::Paused;
            let path = state
                .current_track
                .as_ref()
                .map(|t| t.path.clone())
                .unwrap_or_default();
            drop(state);

            if is_paused && !path.is_empty() {
                inner.mixer.lock().await.play()?;
                if path.starts_with("spotify:") {
                    inner.stream.lock().await.resume();
                }
                let mut state = inner.state.write().await;
                let track = match state.current_track.clone() {
                    Some(t) => t,
                    None => {
                        warn!("resume: current_track is None despite paused status");
                        drop(state);
                        return Ok(DaemonRes::Error {
                            message: "no current track".into(),
                        });
                    }
                };
                state.play(track.clone())?;
                let time_pos = state.time_pos;
                let duration = state.duration;
                drop(state);
                Daemon::push_event(
                    inner,
                    DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: false,
                        time_pos,
                        duration,
                    },
                );
                Ok(DaemonRes::Ok)
            } else if !path.is_empty() {
                Cmd::play(inner, &path, 0.0, false).await
            } else {
                let state = inner.state.read().await;
                let (queue, cursor) = queue::visible(&state);
                drop(state);
                if !queue.is_empty() {
                    let idx = (cursor as usize).min(queue.len() - 1);
                    Cmd::play(inner, &queue[idx].path, 0.0, false).await
                } else {
                    Ok(DaemonRes::Ok)
                }
            }
        }
    }

    pub async fn pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let pos = {
            let mut mixer = inner.mixer.lock().await;
            mixer.pause()?;
            mixer.current_position()
        };
        // Pause the librespot player too: pausing only the mixer leaves the
        // network stream decoding into a full channel.
        let streaming = inner
            .state
            .read()
            .await
            .current_track
            .as_ref()
            .is_some_and(|t| t.path.starts_with("spotify:"));
        if streaming {
            inner.stream.lock().await.pause();
        }
        let mut state = inner.state.write().await;
        state.pause()?;
        state.time_pos = pos;
        let time_pos = state.time_pos;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::PlaybackPaused { time_pos });
        Ok(DaemonRes::Ok)
    }

    pub async fn stop(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.play_session.fetch_add(1, Ordering::Release);
        inner.stream.lock().await.reset();
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let mut state = inner.state.write().await;
        if state.status != PlaybackStatus::Stopped {
            state.stop()?;
        }
        state.radio_title = None;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::PlaybackStopped);
        Daemon::push_event(inner, DaemonEvent::RadioTitleChanged { title: None });
        Ok(DaemonRes::Ok)
    }

    pub async fn next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if let Some(path) = Daemon::promote_crossfade(inner).await {
            let _ = Daemon::step_next(inner).await;
            Daemon::report_promoted(inner, &path).await;
            return Ok(DaemonRes::Ok);
        }
        *inner.crossfade_loaded_for.lock().await = None;
        *inner.countdown_notified_for.lock().await = None;
        let standby = {
            let state = inner.state.read().await;
            Daemon::next_track(&state)
        };
        if let Some(track) = standby
            && Daemon::try_start_crossfade(inner, &track).await
        {
            return Ok(DaemonRes::Ok);
        }
        let next = match Daemon::step_next(inner).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                let was_playing = inner.state.read().await.status != PlaybackStatus::Stopped;
                if was_playing {
                    Daemon::stop_playback(inner).await;
                } else {
                    Daemon::push_queue_state(inner).await;
                }
                return Ok(DaemonRes::Ok);
            }
            Err(e) => return Err(e),
        };
        let _ = Cmd::play(inner, &next.path, 0.0, true).await?;
        Daemon::push_queue_state(inner).await;
        Ok(DaemonRes::Ok)
    }

    pub async fn prev(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if let Some(path) = Daemon::promote_crossfade(inner).await {
            Daemon::report_promoted(inner, &path).await;
        }
        *inner.countdown_notified_for.lock().await = None;
        let pos = inner.mixer.lock().await.current_position();
        if pos > RESTART_THRESHOLD_SECS {
            return Cmd::seek(inner, 0.0).await;
        }
        let has_current = inner.state.read().await.current_track.is_some();

        let prev = inner.play_history.lock().await.pop();
        match prev {
            Some(HistoryEntry::User(track)) => {
                {
                    let mut state = inner.state.write().await;
                    state.queue.insert(0, track.clone());
                }
                let res = Cmd::play(inner, &track.path, 0.0, true).await?;
                Daemon::push_queue_state(inner).await;
                Ok(res)
            }
            Some(HistoryEntry::Default { index, track }) => {
                {
                    let mut state = inner.state.write().await;
                    if index < state.default_list.len() {
                        state.default_cursor = index;
                    }
                }
                let res = Cmd::play(inner, &track.path, 0.0, true).await?;
                Daemon::push_queue_state(inner).await;
                Ok(res)
            }
            None => {
                if has_current {
                    Cmd::seek(inner, 0.0).await
                } else {
                    Ok(DaemonRes::Ok)
                }
            }
        }
    }

    pub async fn seek(inner: &DaemonInner, pos: f64) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        if state.status == PlaybackStatus::Stopped {
            return Ok(DaemonRes::Ok);
        }
        let was_paused = state.status == PlaybackStatus::Paused;
        let path = state.current_track.as_ref().map(|t| t.path.clone());
        let duration = state.duration;
        drop(state);
        let Some(path) = path else {
            return Ok(DaemonRes::Ok);
        };
        let pos = pos.clamp(0.0, duration.max(0.0));
        tracing::debug!("cmd_seek: requested position={}", pos);
        if path.starts_with("spotify:") {
            return Cmd::seek_stream(inner, &path, pos, duration, was_paused).await;
        }
        let path_owned = path.clone();
        let source =
            tokio::task::spawn_blocking(move || AudioMixer::decode_file_at(&path_owned, pos))
                .await
                .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
                .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(source, pos)?;
            mixer.play()?;
            if was_paused {
                mixer.pause()?;
            }
        }
        let mut state = inner.state.write().await;
        state.seek(pos)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::PositionChanged { time_pos: pos });
        Ok(DaemonRes::Ok)
    }

    /// Reload a streamed Spotify track at the new position. Used when the
    /// in-place player seek is unavailable (no live session), since librespot
    /// otherwise re-decodes from the nearest chunk boundary.
    async fn seek_stream(
        inner: &DaemonInner,
        uri_path: &str,
        pos: f64,
        total_duration: f64,
        was_paused: bool,
    ) -> Result<DaemonRes, CoreError> {
        // Prefer an in-place player seek: it keeps the buffer and avoids a full
        // reconnect. Fall through to the reload when the session is gone.
        {
            let mut stream = inner.stream.lock().await;
            if !stream.is_dead() && stream.seek((pos.max(0.0) * 1000.0) as u32) {
                drop(stream);
                if was_paused {
                    inner.stream.lock().await.pause();
                }
                let mut state = inner.state.write().await;
                state.seek(pos)?;
                drop(state);
                Daemon::push_event(inner, DaemonEvent::PositionChanged { time_pos: pos });
                return Ok(DaemonRes::Ok);
            }
        }

        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        let token = match access_token(&client).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(DaemonRes::Error {
                    message: format!("{e}; re-link the account"),
                });
            }
        };
        let config_dir = inner.config.config_dir.clone();
        let source = {
            let mut stream = inner.stream.lock().await;
            match stream
                .load(
                    uri_path,
                    (pos.max(0.0) * 1000.0) as u32,
                    total_duration.max(0.0),
                    &token,
                    &config_dir,
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    return Ok(DaemonRes::Error {
                        message: format!("spotify stream: {e}"),
                    });
                }
            }
        };
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(Box::new(source), pos)?;
            mixer.play()?;
            if was_paused {
                mixer.pause()?;
            }
        }
        let mut state = inner.state.write().await;
        state.seek(pos)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::PositionChanged { time_pos: pos });
        Ok(DaemonRes::Ok)
    }

    pub async fn set_volume(inner: &DaemonInner, volume: u8) -> Result<DaemonRes, CoreError> {
        inner.mixer.lock().await.set_volume(volume)?;
        let mut state = inner.state.write().await;
        state.set_volume(volume)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::VolumeChanged { volume });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn get_volume(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let volume = state.volume;
        drop(state);
        Ok(DaemonRes::Value {
            value: serde_json::json!({ "volume": volume }),
        })
    }

    pub async fn list_eq_presets(_inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let presets = EQ_PRESETS
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<String>>();
        Ok(DaemonRes::EqPresets { presets })
    }

    pub async fn toggle_shuffle(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        let was_shuffled = state.shuffle;
        state.toggle_shuffle()?;
        let enabled = state.shuffle;

        if enabled && !was_shuffled && !state.queue.is_empty() {
            fastrand::shuffle(&mut state.queue);
        }

        drop(state);
        Daemon::push_event(inner, DaemonEvent::ShuffleChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_repeat_mode(
        inner: &DaemonInner,
        mode: RepeatMode,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_repeat_mode(mode)?;
        let m = state.repeat;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::RepeatModeChanged { mode: m });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn toggle_mute(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.toggle_mute()?;
        let muted = state.mute;
        drop(state);
        let vol = if muted {
            0
        } else {
            inner.state.read().await.volume
        };
        inner.mixer.lock().await.set_volume(vol)?;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_mono(inner: &DaemonInner, enabled: bool) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.mono = enabled;
        drop(state);
        inner.mixer.lock().await.set_mono(enabled);
        Daemon::push_event(inner, DaemonEvent::MonoChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn crossfade(
        inner: &DaemonInner,
        enabled: bool,
        duration_secs: u8,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_crossfade(enabled, duration_secs)?;
        drop(state);
        Daemon::push_event(
            inner,
            DaemonEvent::CrossfadeChanged {
                enabled,
                duration_secs,
            },
        );
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_loudness_mode(
        inner: &DaemonInner,
        mode: LoudnessMode,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_loudness_mode(mode)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::LoudnessModeChanged { mode });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn scan_loudness(
        inner: &DaemonInner,
        track_ids: Option<Vec<i64>>,
        _force: Option<bool>,
    ) -> Result<DaemonRes, CoreError> {
        let total = track_ids.as_ref().map(|v| v.len() as u32).unwrap_or(0);
        for i in 0..total {
            let remaining = total - i;
            Daemon::push_event(
                inner,
                DaemonEvent::LoudnessScanProgress {
                    tracks_remaining: remaining,
                    tracks_total: total,
                    current_track: None,
                },
            );
        }
        Daemon::push_event(
            inner,
            DaemonEvent::LoudnessScanDone {
                scanned: total,
                failed: 0,
            },
        );
        Ok(DaemonRes::Ok)
    }

    pub async fn set_pre_gain(
        inner: &DaemonInner,
        pre_gain_db: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_pre_gain(pre_gain_db)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::PreGainChanged { pre_gain_db });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_gapless(inner: &DaemonInner, enabled: bool) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_gapless(enabled)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::GaplessChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_dynamic_mode(
        inner: &DaemonInner,
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_dynamic_mode(enabled, min_queue_remaining, max_history)?;
        let effective_min = min_queue_remaining.unwrap_or(state.dynamic_mode.min_queue_remaining);
        let effective_max = max_history.unwrap_or(state.dynamic_mode.max_history);
        drop(state);
        Daemon::push_event(
            inner,
            DaemonEvent::DynamicModeChanged {
                enabled,
                min_queue_remaining: effective_min,
                max_history: effective_max,
            },
        );
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_scrobble(
        inner: &DaemonInner,
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_scrobble(enabled, api_key, session_token, min_play_secs, min_play_pct)?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::ScrobbleConfigChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_eq_preset(
        inner: &DaemonInner,
        preset: EqPreset,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.audio.eq_preset = preset;
        state.version += 1;
        drop(state);
        inner.mixer.lock().await.set_eq_preset(&preset);
        Daemon::push_event(inner, DaemonEvent::EqPresetChanged { preset });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_eq_enabled(
        inner: &DaemonInner,
        enabled: bool,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.audio.eq_enabled = enabled;
        state.version += 1;
        drop(state);
        inner.mixer.lock().await.set_eq_enabled(enabled);
        Daemon::push_event(inner, DaemonEvent::EqEnabledChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_reverb(
        inner: &DaemonInner,
        enabled: bool,
        room_size: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.audio.reverb = ReverbConfig { enabled, room_size };
        state.version += 1;
        drop(state);
        inner
            .mixer
            .lock()
            .await
            .set_reverb(&ReverbConfig { enabled, room_size });
        Daemon::push_event(inner, DaemonEvent::ReverbChanged { enabled, room_size });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_speed(inner: &DaemonInner, rate: f32) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_speed(rate)?;
        let applied = state.audio.speed;
        drop(state);
        inner.mixer.lock().await.set_speed(applied);
        Daemon::push_event(inner, DaemonEvent::SpeedChanged { rate: applied });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn get_speed(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let speed = state.audio.speed;
        drop(state);
        Ok(DaemonRes::Value {
            value: serde_json::json!({ "speed": speed }),
        })
    }

    pub async fn list_audio_devices(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let devices = inner.mixer.lock().await.list_devices();
        Ok(DaemonRes::Value {
            value: serde_json::json!({ "devices": devices }),
        })
    }

    /// Switch the active output device (`None` restores the system default).
    /// Switching restarts the output and stops playback, so report that via
    /// the status event; the new device name is persisted in audio settings.
    pub async fn set_audio_device(
        inner: &DaemonInner,
        name: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
            let speed = inner.state.read().await.audio.speed;
            mixer.set_speed(speed);
        }
        {
            let mut state = inner.state.write().await;
            state.audio.audio_device = name.clone();
            state.version += 1;
        }
        Daemon::stop_playback(inner).await;
        Daemon::push_event(inner, DaemonEvent::AudioDeviceChanged { name });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn set_sleep_timer(
        inner: &Arc<DaemonInner>,
        minutes: u32,
        stop_immediately: bool,
    ) -> Result<DaemonRes, CoreError> {
        let total_secs = minutes * 60;
        // Invalidate any previously scheduled timer before arming a new one;
        // overlap would otherwise let both loops drive `sleep_timer`.
        let timer_gen = inner
            .sleep_gen
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        // Re-arming supersedes any pending stop-at-track-end from the previous
        // timer.
        inner.sleep_stop_at_track_end.store(false, Ordering::SeqCst);
        let event_tx = inner.event_tx.clone();
        let state = inner.state.clone();

        {
            let mut s = state.write().await;
            s.sleep_timer = Some(total_secs);
            s.version += 1;
        }

        let inner = Arc::clone(inner);
        tokio::spawn(async move {
            for remaining in (1..=total_secs).rev() {
                if inner.sleep_gen.load(Ordering::SeqCst) != timer_gen {
                    return;
                }
                {
                    let mut s = state.write().await;
                    s.sleep_timer = Some(remaining);
                }
                let _ = event_tx.send(DaemonEvent::SleepTimerTick {
                    remaining_secs: remaining,
                });
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if inner.sleep_gen.load(Ordering::SeqCst) != timer_gen {
                return;
            }
            // With "stop immediately" unchecked and a finite track playing, the
            // timer defers: playback keeps running until that track ends
            // naturally, then `AudioEvent::Finished` performs the stop. Radio
            // and bare stream URLs have no end, so they always stop now.
            let (endless, status) = {
                let s = state.read().await;
                (
                    s.current_track
                        .as_ref()
                        .map(|t| Daemon::path_is_live_stream(&t.path))
                        .unwrap_or(true),
                    s.status,
                )
            };
            if !stop_immediately && !endless && status == PlaybackStatus::Playing {
                inner.sleep_stop_at_track_end.store(true, Ordering::SeqCst);
                {
                    let mut s = state.write().await;
                    s.sleep_timer = None;
                    s.version += 1;
                }
                let _ = event_tx.send(DaemonEvent::Custom {
                    name: "sleep_timer_deferred".into(),
                    data: std::collections::HashMap::from([(
                        "note".into(),
                        "Stopping at end of current track".into(),
                    )]),
                });
                return;
            }
            // Expiry must actually silence the output, not just flip the
            // status flag: stop the mixer and any Web (Spotify) stream, then
            // report the state change.
            Daemon::sleep_timer_expiry_stop(&inner).await;
        });

        Ok(DaemonRes::Ok)
    }

    pub async fn cancel_sleep_timer(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        // Bump the generation so the armed countdown (if any) backs out on its
        // next tick instead of stopping playback underneath us.
        inner.sleep_gen.fetch_add(1, Ordering::SeqCst);
        inner.sleep_stop_at_track_end.store(false, Ordering::SeqCst);
        let mut state = inner.state.write().await;
        state.sleep_timer = None;
        state.version += 1;
        Ok(DaemonRes::Ok)
    }

    /// Enter/leave low-power mode. Enabling pauses playback (mirroring the
    /// sleep-timer expiry path) and cancels any armed sleep timer; disabling
    /// simply clears the flag so the user can play again.
    pub async fn set_low_power(inner: &DaemonInner, enabled: bool) -> Result<DaemonRes, CoreError> {
        {
            let mut state = inner.state.write().await;
            if state.low_power == enabled {
                return Ok(DaemonRes::Ok);
            }
            state.set_low_power(enabled)?;
        }
        inner.sleep_gen.fetch_add(1, Ordering::SeqCst);
        inner.sleep_stop_at_track_end.store(false, Ordering::SeqCst);
        if enabled {
            let was_playing = {
                let state = inner.state.read().await;
                state.status == PlaybackStatus::Playing
            };
            if was_playing {
                Cmd::pause(inner).await?;
            }
        }
        Daemon::push_event(inner, DaemonEvent::LowPowerChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Report the current low-power mode.
    pub async fn get_low_power(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let low_power = state.low_power;
        drop(state);
        Ok(DaemonRes::Value {
            value: serde_json::json!({ "low_power": low_power }),
        })
    }

    pub async fn clear_cache(inner: &DaemonInner, what: CacheKind) -> Result<DaemonRes, CoreError> {
        let cache_dir = inner.config.cache_dir.clone();
        tokio::task::spawn_blocking(move || match what {
            CacheKind::Lyrics => {
                let dir = cache_dir.join("lyrics");
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        if entry.path().extension().is_some_and(|x| x == "lrc") {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
            CacheKind::Covers => {
                for sub in ["covers", "artist_covers"] {
                    let dir = cache_dir.join(sub);
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        })
        .await
        .map_err(|e| CoreError::Daemon(format!("clear cache task: {e}")))?;
        if what == CacheKind::Covers {
            // Drop the in-memory LRU too, otherwise the UI reports a cleared
            // cache while the daemon keeps serving the old images from RAM.
            let cache = inner.cover_cache().await;
            if let Some(cc) = cache.as_ref() {
                cc.clear_mem().await;
            }
        }
        Ok(DaemonRes::Ok)
    }

    pub async fn get_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let mut state_clone = state.clone();
        let (queue, cursor) = queue::visible(&state);
        state_clone.queue = queue;
        state_clone.queue_cursor = cursor;
        drop(state);
        Ok(DaemonRes::Status {
            state: Box::new(state_clone),
        })
    }

    /// Like [`get_status`] but drops `default_list` (the whole library) from
    /// the returned state. The client never reads `default_list` — the merged
    /// `queue` view is what it renders — so the periodic background refresh
    /// saves re-serializing the full library on every tick.
    pub async fn get_status_lite(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let mut state_clone = state.clone();
        let (queue, cursor) = queue::visible(&state);
        state_clone.queue = queue;
        state_clone.queue_cursor = cursor;
        state_clone.default_list.clear();
        drop(state);
        Ok(DaemonRes::Status {
            state: Box::new(state_clone),
        })
    }

    pub async fn check_health(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let h = &inner.health;
        let mut components = Vec::new();

        components.push(ComponentHealth {
            name: "audio_backend".into(),
            status: HealthStatus::Ok,
            message: Some(h.audio_backend.clone()),
            uptime_secs: Some(h.uptime_secs()),
        });

        let scans = h.scan.count.load(Ordering::Relaxed);
        let scan_errs = h.scan.errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "library_scan".into(),
            status: if scan_errs > 0 && scans > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{scans} scans, {scan_errs} errors")),
            uptime_secs: None,
        });

        #[cfg(feature = "youtube")]
        {
            let yt = h.yt.count.load(Ordering::Relaxed);
            let yt_errs = h.yt.errors.load(Ordering::Relaxed);
            components.push(ComponentHealth {
                name: "youtube_search".into(),
                status: if yt_errs > 0 && yt > 0 {
                    HealthStatus::Degraded
                } else {
                    HealthStatus::Ok
                },
                message: Some(format!("{yt} searches, {yt_errs} errors")),
                uptime_secs: None,
            });
        }

        let covers = h.cover.count.load(Ordering::Relaxed);
        let cover_errs = h.cover.errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "cover_art".into(),
            status: if cover_errs > 0 && covers > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{covers} fetches, {cover_errs} errors")),
            uptime_secs: None,
        });

        let lyrics = h.lyrics.count.load(Ordering::Relaxed);
        let lyrics_errs = h.lyrics.errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "lyrics".into(),
            status: if lyrics_errs > 0 && lyrics > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{lyrics} fetches, {lyrics_errs} errors")),
            uptime_secs: None,
        });

        components.push(ComponentHealth {
            name: "event_channel".into(),
            status: HealthStatus::Ok,
            message: Some("capacity 1024".to_string()),
            uptime_secs: None,
        });

        Ok(DaemonRes::HealthReport {
            report: Box::new(HealthReport {
                daemon_uptime_secs: h.uptime_secs(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                components,
            }),
        })
    }
}

pub(crate) struct DaemonInner {
    state: Arc<RwLock<DaemonState>>,
    mixer: tokio::sync::Mutex<Box<dyn Mixer>>,
    config: DaemonConfig,
    /// Runtime cover-provider override. `None` means the value baked into
    /// `config` (from config.toml at startup) applies; the TUI switches it
    /// live via `DaemonReq::SetCoverProvider` without a restart.
    cover_provider_override: tokio::sync::Mutex<Option<CoverProvider>>,
    event_tx: broadcast::Sender<DaemonEvent>,
    cover_cache: tokio::sync::Mutex<Option<CoverCache>>,
    lyrics_manager: tokio::sync::Mutex<Option<LyricsManager>>,
    lastfm: tokio::sync::Mutex<LastfmManager>,
    /// Last.fm loved-state for the current track: `Some((artist|title, loved))`
    /// once the user has loved/unloved anything this session.
    lastfm_loved: std::sync::Mutex<Option<(String, bool)>>,
    #[cfg(feature = "youtube")]
    youtube: Arc<tokio::sync::Mutex<YoutubeManager>>,
    spotify: Arc<tokio::sync::Mutex<SpotifyManager>>,
    podcast: tokio::sync::Mutex<PodcastManager>,
    radio: tokio::sync::Mutex<RadioBrowserManager>,
    /// Chart providers registry (Spotify first; more sources plug in via the
    /// same `ChartProvider` trait).
    charts: tokio::sync::Mutex<ChartsRegistry>,
    /// librespot streaming bridge for Premium Spotify playback.
    stream: tokio::sync::Mutex<StreamManager>,
    /// Pending OAuth link flow task; aborted when a new flow starts or the
    /// user cancels.
    oauth_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Pending Last.fm OAuth link flow task (daemon-hosted loopback). Kept
    /// separate from `oauth_task` so the Spotify and Last.fm flows never abort
    /// each other.
    oauth_lastfm_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Last.fm link failure surfaced through the next status poll (callback
    /// timeout, token-exchange error, …). Cleared on a successful exchange so
    /// the Setup picker can show the reason inline instead of hanging.
    lastfm_error: tokio::sync::Mutex<Option<String>>,
    crossfade_loaded_for: tokio::sync::Mutex<Option<String>>,
    countdown_notified_for: tokio::sync::Mutex<Option<String>>,
    /// Wall-clock instant of the last `PositionChanged` broadcast. The 16ms
    /// poll loop emits `AudioEvent::Position` at ~20 Hz; re-anchoring the
    /// client clock on every one would flood the socket, so position gets
    /// broadcast at most this often. Clients extrapolate smoothly between
    /// broadcasts, keeping the lyric highlight and progress bar tight.
    last_pos_broadcast: tokio::sync::Mutex<Option<std::time::Instant>>,
    /// Latest ICY `StreamTitle` seen on the active live stream. Written by
    /// the decode thread inside the ICY reader; the position tick mirrors it
    /// into `state.radio_title` and broadcasts on change.
    icy_title: remote::IcySlot,
    /// Monotonic generation counter for the sleep timer. `set_sleep_timer`
    /// bumps it so any previously scheduled timer observes the mismatch and
    /// backs out without racing the new one; `cancel_sleep_timer` also bumps.
    sleep_gen: Arc<AtomicU64>,
    /// Set when the sleep timer expires with "stop immediately" disabled and a
    /// finite track is playing. Playback keeps running until that track ends
    /// naturally (`AudioEvent::Finished`), at which point the daemon stops and
    /// reports `SleepTimerExpired` instead of auto-advancing. Cleared whenever
    /// the user re-arms/cancels the timer or manually starts new playback.
    sleep_stop_at_track_end: Arc<AtomicBool>,
    /// Monotonic counter bumped on every play/stop path. Crossfade tasks
    /// capture it at spawn time and abort if it has changed, preventing a
    /// stale auto-advance from overwriting a user-initiated playback switch.
    play_session: Arc<AtomicU64>,
    health: Arc<HealthTracker>,
    active_clients: AtomicUsize,
    internal_req_tx: mpsc::UnboundedSender<DaemonReq>,
    /// Serializes state-mutating commands. Read-only commands take a read
    /// lock so fast reads are not blocked behind slow mutating operations
    /// (Spotify sync, YouTube download, library scan, audio decode).
    cmd_lock: tokio::sync::RwLock<()>,
    /// Serializes fast user-initiated playback commands (play/pause/next/prev/
    /// seek/volume) against each other only. Playback runs on this lock rather
    /// than `cmd_lock` so a slow background job (Spotify sync, yt-dlp, loudness
    /// scan) holding the exclusive `cmd_lock` never delays the remote's next
    /// track. Long-running jobs and playback commands can then interleave: the
    /// underlying `DaemonState` keeps each individual mutation safe.
    play_lock: tokio::sync::RwLock<()>,
    /// Serializes slow Spotify network commands (sync, resolve, album/artist
    /// track fetches) without blocking fast reads: `GetStatus`/`Ping` never
    /// take this lock, so they stay responsive even when a multi-minute
    /// Spotify sync is in progress. A separate `yt_lock` keeps Spotify and
    /// YouTube jobs from serializing each other.
    spotify_slow_lock: tokio::sync::Mutex<()>,
    /// Serializes slow YouTube network commands (search, resolve/download,
    /// playlist fetch) independently of Spotify's slow lock.
    yt_slow_lock: tokio::sync::Mutex<()>,
    play_history: tokio::sync::Mutex<Vec<HistoryEntry>>,
    scrobble: tokio::sync::Mutex<ScrobbleTracker>,
    sync_progress: Arc<SyncProgress>,
}

impl DaemonInner {
    /// Return the cover cache, creating it on first use so the costly
    /// `reqwest::Client` (TLS) init is deferred until a cover is actually
    /// requested instead of at daemon startup.
    async fn cover_cache(&self) -> tokio::sync::MutexGuard<'_, Option<CoverCache>> {
        let mut guard = self.cover_cache.lock().await;
        if guard.is_none() {
            let cache = CoverCache::new(self.config.cache_dir.clone());
            cache.set_disk_cap(self.config.cover_cache_bytes);
            *guard = Some(cache);
        }
        guard
    }

    /// Cover provider in effect right now: the runtime override wins over the
    /// value parsed from config.toml at startup.
    async fn effective_cover_provider(&self) -> CoverProvider {
        (*self.cover_provider_override.lock().await).unwrap_or(self.config.cover_provider)
    }

    /// Return a clone of the lyrics manager, creating it on first use so the
    /// `reqwest::Client` (TLS) init is deferred until lyrics are needed.
    async fn lyrics_manager(&self) -> Option<LyricsManager> {
        let mut guard = self.lyrics_manager.lock().await;
        if guard.is_none() {
            *guard = Some(LyricsManager::with_cache_dir(
                self.config.cache_dir.join("lyrics"),
            ));
        }
        guard.clone()
    }
}

/// Keychain entries owned by streaming providers that gtm no longer supports.
/// Deleting a missing key is a no-op, so calling this on every start is safe.
const RETIRED_PROVIDER_SECRET_KEYS: &[&str] = &[
    "subsonic_credentials",
    "deezer_arl",
    "tidal_client_id",
    "tidal_token",
];

/// Drop keychain entries left behind by removed providers. Their commands and
/// readers are gone, so the values can never be used again and would otherwise
/// sit in the OS keyring (or the config-dir file fallback) indefinitely. This
/// also covers secrets written by older builds before the removal, which no
/// config migration can reach.
fn purge_retired_provider_secrets() {
    for key in RETIRED_PROVIDER_SECRET_KEYS {
        delete_secret(key);
    }
}

pub struct Daemon {
    inner: Arc<DaemonInner>,
    listener: UnixListener,
    pulse_listener: UnixListener,
    req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, u64, DaemonReq, ReplyTx)>,
    internal_req_rx: mpsc::UnboundedReceiver<DaemonReq>,
    next_client_id: ClientId,
    #[cfg(feature = "mpris")]
    mpris: Option<MprisHandle>,
}

/// Returns `true` for requests that only read state and never mutate it, so
/// they can share a read lock and run concurrently with each other instead of
/// being serialized behind slow mutating commands.
fn is_read_only(req: &DaemonReq) -> bool {
    matches!(
        req,
        DaemonReq::GetStatus
            | DaemonReq::GetStatusLite
            | DaemonReq::CheckHealth
            | DaemonReq::Ping
            | DaemonReq::ListEqPresets
            | DaemonReq::GetVolume
            | DaemonReq::GetSpeed
            | DaemonReq::GetLowPower
            | DaemonReq::ListAudioDevices
            | DaemonReq::GetFavourites
            | DaemonReq::GetCoverArt { .. }
            | DaemonReq::GetArtistCoverArt { .. }
            | DaemonReq::GetLyrics { .. }
            | DaemonReq::LyricsSearch { .. }
            | DaemonReq::SpotifyStatus
            | DaemonReq::SpotifyPlaylists
            | DaemonReq::SpotifyPlaylistTracks { .. }
            | DaemonReq::SpotifySearchWeb { .. }
            | DaemonReq::SpotifyAlbumTracks { .. }
            | DaemonReq::SpotifyArtistTopTracks { .. }
            | DaemonReq::SpotifyTrackImage { .. }
            | DaemonReq::LastfmStatus
            | DaemonReq::PodcastFeeds
            | DaemonReq::PodcastEpisodes { .. }
            | DaemonReq::PodcastStatus
            | DaemonReq::RadioSearch { .. }
            | DaemonReq::RadioTop { .. }
            | DaemonReq::RadioTags { .. }
            | DaemonReq::RadioByTag { .. }
            | DaemonReq::RadioCountries { .. }
            | DaemonReq::RadioByCountry { .. }
            | DaemonReq::Search { .. }
            | DaemonReq::Queue {
                action: QueueAction::List,
            }
            | DaemonReq::Library {
                action: LibraryAction::GetTracks { .. }
                    | LibraryAction::GetPlaylists
                    | LibraryAction::GetPlaylistTracks { .. }
                    | LibraryAction::GetRecent { .. }
                    | LibraryAction::SyncStatus,
            }
    )
}

/// Fast user-facing transport commands. These serialize against each other on
/// `play_lock` (cheap, sub-millisecond contention) and deliberately skip the
/// global `cmd_lock` so slow background jobs can never stall next/prev/seek.
fn request_is_playback(req: &DaemonReq) -> bool {
    matches!(
        req,
        DaemonReq::Play { .. }
            | DaemonReq::PlayPause
            | DaemonReq::Pause
            | DaemonReq::Stop
            | DaemonReq::Next
            | DaemonReq::Prev
            | DaemonReq::Seek { .. }
            | DaemonReq::SetVolume { .. }
            | DaemonReq::ToggleMute
            | DaemonReq::SetMono { .. }
            | DaemonReq::SetSpeed { .. }
            | DaemonReq::SetEqPreset { .. }
            | DaemonReq::SetEqEnabled { .. }
            | DaemonReq::SetReverb { .. }
            | DaemonReq::SetPreGain { .. }
            | DaemonReq::SetLoudnessMode { .. }
            | DaemonReq::SetGapless { .. }
            | DaemonReq::SetDynamicMode { .. }
            | DaemonReq::SetLowPower { .. }
            | DaemonReq::SetSleepTimer { .. }
            | DaemonReq::CancelSleepTimer
            | DaemonReq::SetAudioDevice { .. }
            | DaemonReq::SpotifyPlayPause
            | DaemonReq::SpotifyNext
            | DaemonReq::SpotifyPrevious
            | DaemonReq::SpotifySeek { .. }
            | DaemonReq::SpotifyShuffle { .. }
            | DaemonReq::SpotifyRepeat { .. }
            | DaemonReq::SpotifyVolume { .. }
            | DaemonReq::PodcastPlay { .. }
            | DaemonReq::RadioPlay { .. }
    )
}

/// Commands that touch remote APIs and can take many seconds (Spotify sync,
/// Spotify resolve, YouTube search/download). They serialize on per-provider
/// locks instead of `cmd_lock.write()` so fast reads (`GetStatus`/`Ping`)
/// never get stuck behind a multi-minute network stall.  Spotify and YouTube
/// jobs use separate locks so a long Spotify sync no longer blocks a quick
/// YouTube search.
fn is_spotify_slow(req: &DaemonReq) -> bool {
    matches!(
        req,
        DaemonReq::SpotifySync
            | DaemonReq::SpotifyResolve { .. }
            | DaemonReq::SpotifyResolveTrack { .. }
            | DaemonReq::SpotifyAlbumTracks { .. }
            | DaemonReq::SpotifyArtistTopTracks { .. }
            | DaemonReq::SpotifyWebPlaylistTracks { .. }
    )
}

fn is_yt_slow(req: &DaemonReq) -> bool {
    matches!(
        req,
        DaemonReq::YtSearch { .. }
            | DaemonReq::YtResolveStream { .. }
            | DaemonReq::YtDownload { .. }
            | DaemonReq::YtFetchPlaylist { .. }
    )
}

impl Daemon {
    pub async fn new(config: DaemonConfig) -> Result<Self, CoreError> {
        let mut initial_state = DaemonState::new();

        if !config.test_mode
            && let Some(saved) = SavedState::load(&config.state_file)
        {
            info!("loaded saved state from {}", config.state_file.display());
            saved.apply_to(&mut initial_state);
        }

        // The real mixer build (PulseAudio connect, device enumeration, and on
        // Termux possibly spawning the PulseAudio server) is deferred to the
        // first actual mixer call via `DeferredMixer`, so the IPC socket binds
        // before any audio-device or network I/O happens. Persisted device /
        // speed / mono are replayed inside the factory on first init.
        // NOTE: unlike the old eager path, a saved device that disappeared is
        // no longer cleared from `initial_state` here; the factory warns and
        // keeps the system default, and the stale name is retried (and
        // re-warned) on the next launch.
        let mixer: Box<dyn Mixer> = if config.test_mode {
            Box::new(NullMixer::new())
        } else {
            let cfg = config.clone();
            let device = initial_state.audio.audio_device.clone();
            let speed = initial_state.audio.speed;
            let mono = initial_state.mono;
            Box::new(DeferredMixer::new(move || {
                let mut m =
                    Self::init_mixer(&cfg).map_err(|e| AudioError::OutputError(e.to_string()))?;
                if let Some(dev) = device.clone()
                    && let Err(e) = m.set_device(Some(dev.clone()))
                {
                    warn!("saved audio device '{dev}' unavailable ({e}); using default");
                }
                if (speed - 1.0).abs() > f32::EPSILON {
                    m.set_speed(speed);
                }
                if mono {
                    m.set_mono(true);
                }
                Ok(m)
            }))
        };

        let state = Arc::new(RwLock::new(initial_state));

        let socket_path = Path::new(&config.socket_path);
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create socket dir: {e}")))?;
        }
        // Single-instance guard: refuse to steal a live daemon's socket. Only a
        // stale pidfile (dead process) allows the leftover socket file to be
        // cleared and this instance to bind, so concurrent clients can never
        // end up with two daemons.
        if !config.test_mode
            && let Some(pid) = gtm::shared::daemon::read_daemon_pid()
            && gtm::shared::daemon::pid_is_alive(pid)
        {
            return Err(CoreError::Daemon(format!(
                "gtmd already running (pid {pid}); connect to the existing daemon"
            )));
        }
        if socket_path.exists() {
            match std::fs::remove_file(socket_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::Daemon(format!("remove stale socket: {e}"))),
            }
        }

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| CoreError::Daemon(format!("bind socket: {e}")))?;

        let pulse_path = Path::new(&config.socket_pulse_path);
        if let Some(parent) = pulse_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create pulse socket dir: {e}")))?;
        }
        if pulse_path.exists() {
            match std::fs::remove_file(pulse_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::Daemon(format!("remove stale pulse socket: {e}"))),
            }
        }
        let pulse_listener = UnixListener::bind(pulse_path)
            .map_err(|e| CoreError::Daemon(format!("bind pulse socket: {e}")))?;

        let pid_file = resolve_pid_file();
        if let Some(parent) = pid_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&pid_file, std::process::id().to_string()) {
            warn!("failed to write PID file: {e}");
        }

        // Daemon → client event broadcast. Sized generously (4× the original
        // 1024) because the visualizer alone bursts at ~60 Hz on top of
        // position/notification traffic; the lagged-receiver path re-syncs to
        // the newest event, but a small channel overflowed on busy sessions
        // and forced frequent resyncs.
        let (event_tx, _) = broadcast::channel::<DaemonEvent>(EVENT_CHANNEL_CAPACITY);
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (internal_req_tx, internal_req_rx) = mpsc::unbounded_channel();

        let config_dir = config.config_dir.clone();
        let audio_backend_name = match config.audio_backend {
            #[cfg(feature = "pulseaudio")]
            AudioBackendKind::PulseAudio => "pulseaudio",
            AudioBackendKind::Rodio => "rodio",
        };

        let inner = Arc::new(DaemonInner {
            state,
            mixer: tokio::sync::Mutex::new(mixer),
            config,
            event_tx,
            cover_cache: tokio::sync::Mutex::new(None),
            cover_provider_override: tokio::sync::Mutex::new(None),
            lyrics_manager: tokio::sync::Mutex::new(None),
            lastfm: tokio::sync::Mutex::new(LastfmManager::new()),
            lastfm_loved: std::sync::Mutex::new(None),
            #[cfg(feature = "youtube")]
            youtube: Arc::new(tokio::sync::Mutex::new(YoutubeManager::new())),
            spotify: Arc::new(tokio::sync::Mutex::new(SpotifyManager::new(
                config_dir.clone(),
            ))),
            podcast: tokio::sync::Mutex::new(PodcastManager::new(config_dir)),
            radio: tokio::sync::Mutex::new(RadioBrowserManager::new()),
            charts: tokio::sync::Mutex::new(ChartsRegistry::empty()),
            stream: tokio::sync::Mutex::new(StreamManager::new()),
            oauth_task: tokio::sync::Mutex::new(None),
            oauth_lastfm_task: tokio::sync::Mutex::new(None),
            lastfm_error: tokio::sync::Mutex::new(None),
            crossfade_loaded_for: tokio::sync::Mutex::new(None),
            countdown_notified_for: tokio::sync::Mutex::new(None),
            last_pos_broadcast: tokio::sync::Mutex::new(None),
            icy_title: Arc::new(std::sync::Mutex::new(None)),
            sleep_gen: Arc::new(AtomicU64::new(0)),
            sleep_stop_at_track_end: Arc::new(AtomicBool::new(false)),
            play_session: Arc::new(AtomicU64::new(0)),
            health: Arc::new(HealthTracker::new(audio_backend_name)),
            active_clients: AtomicUsize::new(0),
            internal_req_tx,
            cmd_lock: tokio::sync::RwLock::new(()),
            play_lock: tokio::sync::RwLock::new(()),
            spotify_slow_lock: tokio::sync::Mutex::new(()),
            yt_slow_lock: tokio::sync::Mutex::new(()),
            play_history: tokio::sync::Mutex::new(Vec::new()),
            scrobble: tokio::sync::Mutex::new(ScrobbleTracker::default()),
            sync_progress: Arc::new(SyncProgress::default()),
        });

        // Initialize charts registry with Spotify provider if configured.
        // The free (no-auth) providers are always registered so Top Charts
        // works even before any account is linked.
        {
            let mut charts = inner.charts.lock().await;
            charts.add_free_defaults();
            let spotify_mgr = inner.spotify.lock().await;
            if spotify_mgr.linked() {
                charts.add_spotify(inner.spotify.clone());
            }
        }

        Ok(Self {
            inner,
            listener,
            pulse_listener,
            req_tx,
            req_rx,
            internal_req_rx,
            next_client_id: 0,
            #[cfg(feature = "mpris")]
            mpris: None,
        })
    }

    #[cfg(feature = "pulseaudio")]
    fn init_mixer(config: &DaemonConfig) -> Result<Box<dyn Mixer>, CoreError> {
        if config.audio_backend == AudioBackendKind::PulseAudio {
            match PulseAudioMixer::new() {
                Ok(m) => Ok(Box::new(m)),
                Err(e) => {
                    if is_termux() {
                        // "All the user has to do is run gtm": try to launch the
                        // PulseAudio server before giving up, so no manual
                        // `pulseaudio --start` is required.
                        ensure_termux_pulse();
                        match PulseAudioMixer::new() {
                            Ok(m) => Ok(Box::new(m)),
                            Err(retry) => Err(CoreError::Daemon(format!(
                                "PulseAudio init failed: {retry}. Auto-start attempted but \
                                 the server is still unavailable. Ensure audio is routed \
                                 (export PULSE_SERVER=127.0.0.1 if over TCP)."
                            ))),
                        }
                    } else {
                        warn!("PulseAudio init failed ({e}), falling back to rodio");
                        AudioMixer::new()
                            .map(|m| Box::new(m) as Box<dyn Mixer>)
                            .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
                    }
                }
            }
        } else {
            AudioMixer::new()
                .map(|m| Box::new(m) as Box<dyn Mixer>)
                .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
        }
    }

    #[cfg(not(feature = "pulseaudio"))]
    fn init_mixer(_config: &DaemonConfig) -> Result<Box<dyn Mixer>, CoreError> {
        AudioMixer::new()
            .map(|m| Box::new(m) as Box<dyn Mixer>)
            .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!(
            "daemon started on {} (pulse: {})",
            self.inner.config.socket_path.display(),
            self.inner.config.socket_pulse_path.display()
        );

        let bg_state = self.inner.state.clone();
        let bg_lib_paths = self.inner.config.library_paths.clone();
        let bg_data_dir = self.inner.config.data_dir.clone();
        let bg_cache_dir = self.inner.config.cache_dir.clone();
        let bg_req_tx = self.req_tx.clone();
        let bg_event_tx = self.inner.event_tx.clone();
        let bg_health = self.inner.health.clone();
        tokio::spawn(async move {
            Self::background_scan(
                bg_state,
                bg_lib_paths,
                bg_data_dir,
                bg_cache_dir,
                bg_req_tx,
                bg_event_tx,
                bg_health,
            )
            .await;
        });

        let hb_event_tx = self.inner.event_tx.clone();
        tokio::spawn(async move {
            // 15s interval against the client's 60s timeout: up to three beats
            // can stall (slow yt-dlp spawn, event-loop hiccup) before the TUI
            // treats the daemon as stale and fails in-flight requests.
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                let _ = hb_event_tx.send(DaemonEvent::Heartbeat);
            }
        });

        #[cfg(feature = "mpris")]
        if !self.inner.config.test_mode {
            let mpris_state = self.inner.state.clone();
            let mpris_event_rx = self.inner.event_tx.subscribe();
            let mpris_req_tx = self.inner.internal_req_tx.clone();
            match start(mpris_state, mpris_event_rx, mpris_req_tx).await {
                Ok(handle) => self.mpris = Some(handle),
                Err(e) => warn!("mpris: failed to start D-Bus server: {e}"),
            }
        }

        let spotify_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            // Local-only link: read the token file and build the client with
            // no network I/O, so startup never stalls on a stalled network
            // while holding the manager mutex. The playlist sync runs below
            // in the background with retry-forever backoff.
            let linked = {
                let mut spotify = spotify_inner.spotify.lock().await;
                if !spotify.has_token_file() {
                    false
                } else {
                    match spotify.load().await {
                        Ok(()) => {
                            info!("spotify client ready on startup");
                            true
                        }
                        Err(e) => {
                            warn!("spotify startup link failed: {e}");
                            spotify.set_error(format!("startup link failed: {e}"));
                            false
                        }
                    }
                }
            };
            if linked {
                // The charts registry is built before any token is loaded, so
                // register the Spotify provider now that a client exists.
                {
                    let mut charts = spotify_inner.charts.lock().await;
                    charts.ensure_spotify(spotify_inner.spotify.clone());
                }
                let _ = spotify_inner
                    .event_tx
                    .send(DaemonEvent::SpotifyStatusChanged);
                // Background playlist sync with retry-forever backoff: the
                // first attempt after boot routinely fails (no network yet,
                // sleeping laptop) and must never give up — a later recovery
                // auto-heals via the success event below. The manager mutex
                // is only ever held for the brief client clone / commit swap,
                // never across the network pass.
                let sync_inner = Arc::clone(&spotify_inner);
                tokio::spawn(async move {
                    let mut delay_secs: u64 = 5;
                    loop {
                        let client = { sync_inner.spotify.lock().await.client() };
                        let Some(client) = client else {
                            // Unlinked while backing off; do not resurrect.
                            return;
                        };
                        match SpotifyManager::run_sync(client).await {
                            Ok((user, playlists)) => {
                                {
                                    let mut spotify = sync_inner.spotify.lock().await;
                                    if spotify.linked() {
                                        let count = playlists.len();
                                        spotify.commit_sync(user, playlists);
                                        info!("spotify playlists synced ({count} playlists)");
                                    }
                                }
                                // Bounded playback probe outside the commit
                                // lock so `/me/player` never blocks other
                                // Spotify commands.
                                {
                                    let mut spotify = sync_inner.spotify.lock().await;
                                    if spotify.linked() {
                                        let _ = tokio::time::timeout(
                                            Duration::from_secs(10),
                                            spotify.refresh_playback(),
                                        )
                                        .await;
                                    }
                                }
                                let _ = sync_inner.event_tx.send(DaemonEvent::SpotifyStatusChanged);
                                return;
                            }
                            Err(e) => {
                                warn!(
                                    "spotify startup sync failed: {e} — retrying in {delay_secs}s"
                                );
                                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                                delay_secs = (delay_secs * 2).min(300);
                            }
                        }
                    }
                });
            } else {
                // The TUI only refreshes its Spotify pane when it hears this
                // event, so re-announce even when nothing linked; otherwise
                // the pane stays stale until another event happens.
                let _ = spotify_inner
                    .event_tx
                    .send(DaemonEvent::SpotifyStatusChanged);
            }
        });

        // Generic connectivity probes for the footer `Network` module. Always
        // on from first boot (independent of any provider link), generic hosts
        // only, bounded per probe; low-power mode pauses probing. Broadcasts
        // only on change so the event channel stays quiet when stable.
        if !self.inner.config.test_mode {
            let net_inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut last: Option<bool> = None;
                loop {
                    interval.tick().await;
                    if net_inner.state.read().await.low_power {
                        continue;
                    }
                    let online = network::probe_online().await;
                    if last != Some(online) {
                        last = Some(online);
                        {
                            let mut state = net_inner.state.write().await;
                            state.network_online = Some(online);
                        }
                        let _ = net_inner
                            .event_tx
                            .send(DaemonEvent::NetworkStatusChanged { online });
                    }
                }
            });
        }

        purge_retired_provider_secrets();

        let provider_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut podcast = provider_inner.podcast.lock().await;
            podcast.load();
            drop(podcast);
            Lastfm::restore_credentials(&provider_inner).await;
            Lastfm::set_retry_queue(&provider_inner).await;
            // Periodically flush any scrobbles that failed transiently (network
            // drop, server 5xx). Runs in the background so retries never block
            // playback; the queue is persisted on disk and survives restarts.
            let flush_inner = Arc::clone(&provider_inner);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(300));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    let lastfm = flush_inner.lastfm.lock().await;
                    lastfm.flush_retries().await;
                }
            });
        });

        // Resume exactly as the user left it: if the saved state carried a
        // `current_track`, start playback (or restore paused) at the saved
        // position. A failed resume (e.g. missing file) clears the ghost
        // entry so the TUI doesn't show a stale track.
        let resume_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let (path, start_pos, was_playing) = {
                let state = resume_inner.state.read().await;
                match &state.current_track {
                    Some(t) => {
                        let playing = state.status == PlaybackStatus::Playing;
                        (t.path.clone(), state.time_pos, playing)
                    }
                    None => return,
                }
            };
            if let Err(e) = Cmd::play(&resume_inner, &path, start_pos, false).await {
                warn!("failed to resume last track at startup: {e}");
                let mut state = resume_inner.state.write().await;
                state.current_track = None;
                state.time_pos = 0.0;
                state.status = PlaybackStatus::Stopped;
            } else if !was_playing {
                let _ = Cmd::pause(&resume_inner).await;
            }
        });

        let mut poll_interval = tokio::time::interval(Duration::from_millis(16));
        let mut save_interval = tokio::time::interval(Duration::from_secs(60));
        let mut last_spectrum_tx = tokio::time::Instant::now();
        let mut last_wave_tx = tokio::time::Instant::now();
        let mut last_started_path: Option<String> = None;
        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    let result = { self.inner.mixer.lock().await.poll() };
                    Self::handle_audio_event(&self.inner, result).await;
                    // Record a listen the first poll tick that observes a
                    // going-to-play track (path changed mid-playback).
                    let started = {
                        let s = self.inner.state.read().await;
                        if s.status == PlaybackStatus::Playing {
                            s.current_track.as_ref().cloned()
                        } else {
                            None
                        }
                    };
                    if let Some(t) = &started {
                        let path = t.path.clone();
                        if last_started_path.as_deref() != Some(path.as_str()) {
                            last_started_path = Some(path);
                            Self::record_play(&self.inner, t).await;
                        }
                    }
                    // Disable the visualizer (spectrum analysis + broadcast)
                    // when no TUI client is connected to conserve CPU.
                    if self.inner.active_clients.load(Ordering::Relaxed) > 0 {
                        // Publish streamed-source spectrum (local files feed the
                        // analyzer from the decode thread; streams from the
                        // rodio source itself).
                        let levels = self.inner.stream.lock().await.spectrum_snapshot();
                        let spectrum = {
                            let mixer = self.inner.mixer.lock().await;
                            if !levels.is_empty() {
                                mixer.publish_spectrum(levels);
                            }
                            mixer.current_spectrum()
                        };
                        {
                            let mut state = self.inner.state.write().await;
                            if spectrum.is_empty() {
                                state.audio_levels.clear();
                            } else {
                                state.audio_levels = spectrum.clone();
                            }
                        }
                        // Throttle visualizer spectrum broadcast to ~60 Hz.
                        if !spectrum.is_empty()
                            && last_spectrum_tx.elapsed() >= Duration::from_millis(16)
                        {
                            last_spectrum_tx = tokio::time::Instant::now();
                            Self::push_event(
                                &self.inner,
                                DaemonEvent::SpectrumChanged { levels: spectrum },
                            );
                        }
                        // Publish streamed-source waveform (streams bypass the
                        // decode thread; local files feed the ring from the
                        // decode thread itself). Mirror of the spectrum flow.
                        {
                            let stream = self.inner.stream.lock().await;
                            let (ws, st) = stream.waveform_snapshot();
                            if !ws.is_empty() {
                                let mixer = self.inner.mixer.lock().await;
                                mixer.publish_waveform(ws, st);
                            }
                        }
                        let (wave_samples, wave_stereo) = {
                            let mixer = self.inner.mixer.lock().await;
                            mixer.current_waveform()
                        };
                        {
                            let mut state = self.inner.state.write().await;
                            state.wave_samples = wave_samples.clone();
                            state.wave_stereo = wave_stereo;
                        }
                        // Throttle visualizer waveform broadcast to ~30 Hz
                        // (the oscilloscope window is ~93 ms; 30 Hz is ample).
                        if !wave_samples.is_empty()
                            && last_wave_tx.elapsed() >= Duration::from_millis(32)
                        {
                            last_wave_tx = tokio::time::Instant::now();
                            Self::push_event(
                                &self.inner,
                                DaemonEvent::WaveformChanged {
                                    samples: wave_samples,
                                    stereo: wave_stereo,
                                },
                            );
                        }
                    }
                }
                _ = save_interval.tick() => {
                    Self::save_state(&self.inner);
                }
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let client_id = self.next_client_id;
                            self.next_client_id += 1;
                            let inner = Arc::clone(&self.inner);
                            let req_tx = self.req_tx.clone();
                            tokio::spawn(async move {
                                Self::accept_client(client_id, stream, inner, req_tx).await;
                            });
                        }
                        Err(e) => {
                            error!("accept failed: {e}");
                        }
                    }
                }
                result = self.pulse_listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let inner = Arc::clone(&self.inner);
                            tokio::spawn(async move {
                                Self::accept_pulse_client(stream, &inner).await;
                            });
                        }
                        Err(e) => {
                            error!("pulse accept failed: {e}");
                        }
                    }
                }
                Some(req) = self.internal_req_rx.recv() => {
                    // Run internal commands on a detached task rather than
                    // inline on the select loop: a slow internal command
                    // (spotify sync, crossfade finish -> decode, scrobble)
                    // must never freeze client acceptance or queued responses.
                    // Playback internals (auto-advance next) use the dedicated
                    // play lock so a background job can't stall track changes.
                    let inner = Arc::clone(&self.inner);
                    tokio::spawn(async move {
                        if request_is_playback(&req) {
                            let _lock = inner.play_lock.write().await;
                            if let Err(e) = Self::handle_request(&inner, &req, 0).await {
                                warn!("internal command {:?} failed: {e}", req);
                            }
                        } else {
                            let _lock = inner.cmd_lock.write().await;
                            if let Err(e) = Self::handle_request(&inner, &req, 0).await {
                                warn!("internal command {:?} failed: {e}", req);
                            }
                        }
                    });
                }
                Some((client_id, request_id, req, reply_tx)) = self.req_rx.recv() => {
                    let inner = Arc::clone(&self.inner);
                    tokio::spawn(async move {
                        Self::dispatch(inner, client_id, request_id, req, reply_tx).await;
                    });
                }
            }
        }
    }

    async fn background_scan(
        state: Arc<RwLock<DaemonState>>,
        library_paths: Vec<std::path::PathBuf>,
        data_dir: std::path::PathBuf,
        cache_dir: std::path::PathBuf,
        _req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
        _event_tx: broadcast::Sender<DaemonEvent>,
        health: Arc<HealthTracker>,
    ) {
        if library_paths.is_empty() {
            return;
        }
        let total_tracks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for audio_dir in &library_paths {
            if !audio_dir.exists() {
                info!("library path {:?} does not exist: skipping", audio_dir);
                continue;
            }
            let audio_dir_str = audio_dir.to_string_lossy().to_string();
            let data_dir = data_dir.clone();
            let cache_dir_str = cache_dir.to_string_lossy().to_string();
            let total = total_tracks.clone();
            let result = tokio::task::spawn_blocking(move || {
                let lib = match Library::new(data_dir.to_str().unwrap_or("")) {
                    Ok(l) => l,
                    Err(e) => return Err(format!("Library::new: {e}")),
                };
                lib.scan_directory(&audio_dir_str, true, Some(&cache_dir_str))
                    .map_err(|e| format!("scan: {e}"))
            })
            .await;
            let tracks = match result {
                Ok(Ok(t)) => {
                    health.scan.count.fetch_add(1, Ordering::Relaxed);
                    t
                }
                Ok(Err(e)) => {
                    health.scan.errors.fetch_add(1, Ordering::Relaxed);
                    warn!("auto-scan {:?} failed: {e}", audio_dir);
                    continue;
                }
                Err(e) => {
                    health.scan.errors.fetch_add(1, Ordering::Relaxed);
                    warn!("auto-scan task panicked for {:?}: {e}", audio_dir);
                    continue;
                }
            };
            let count = tracks.len();
            if count == 0 {
                info!("auto-scan found no new tracks in {:?}", audio_dir);
                continue;
            }
            info!("auto-scanned {} track(s) from {:?}", count, audio_dir);
            total.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
            let mut s = state.write().await;
            for track in &tracks {
                s.queue.push(track.clone());
            }
            drop(s);
        }
    }

    async fn accept_client(
        client_id: ClientId,
        stream: UnixStream,
        inner: Arc<DaemonInner>,
        req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    ) {
        inner.active_clients.fetch_add(1, Ordering::Relaxed);

        let (reader, writer) = stream.into_split();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<(u64, DaemonRes)>();

        let token = tokio_util::sync::CancellationToken::new();

        let r_tx = reply_tx.clone();
        let inner_clone = inner.clone();
        let token_reader = token.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                tokio::select! {
                    _ = token_reader.cancelled() => break,
                    result = reader.read_line(&mut line) => {
                        match result {
                            Ok(0) => break,
                            Ok(_) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    line.clear();
                                    continue;
                                }
                                if trimmed.len() > 1_048_576 {
                                    warn!("client {client_id}: line too long ({} bytes), disconnecting", trimmed.len());
                                    break;
                                }
                                let wire_req: WireReq = match serde_json::from_str(trimmed) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        warn!("client {client_id} malformed JSON, closing: {e}");
                                        break;
                                    }
                                };
                                let WireReq { id, cmd, params } = wire_req;
                                let daemon_req = match DaemonReq::parse_cmd(&cmd, params) {
                                    Ok(r) => r,
                                    Err(e) if e.starts_with("unknown command:") => {
                                        let _ = r_tx.send((id, DaemonRes::Error {
                                            message: e,
                                        }));
                                        line.clear();
                                        continue;
                                    }
                                    Err(e) => {
                                        let _ = r_tx.send((id, DaemonRes::Error {
                                            message: format!("invalid params for {cmd}: {e}"),
                                        }));
                                        line.clear();
                                        continue;
                                    }
                                };
                                if req_tx.send((client_id, id, daemon_req, r_tx.clone())).is_err() {
                                    break;
                                }
                                line.clear();
                            }
                            Err(e) => {
                                warn!("client {client_id} read error: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            token_reader.cancel();
            inner_clone.active_clients.fetch_sub(1, Ordering::Relaxed);
            info!("client {client_id} disconnected");
            if inner_clone.active_clients.load(Ordering::Relaxed) == 0 {
                Daemon::maybe_prune_idle(&inner_clone).await;
            }
        });

        tokio::spawn(async move {
            let mut writer = writer;
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    res = reply_rx.recv() => {
                        match res {
                            Some((id, response)) => {
                                let line = match response.to_wire_line(id) {
                                    Ok(line) => line,
                                    Err(e) => {
                                        warn!("serialize response: {e}");
                                        continue;
                                    }
                                };
                                let line = line + "\n";
                                if writer.write_all(line.as_bytes()).await.is_err()
                                    || writer.flush().await.is_err()
                                {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        info!("client {client_id} connected");
    }

    async fn accept_pulse_client(stream: UnixStream, inner: &DaemonInner) {
        let event_rx = inner.event_tx.subscribe();
        tokio::spawn(async move {
            let mut writer = stream;
            let mut event_rx = event_rx;
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let frame = match wire::encode(&[event]) {
                            Ok(f) => f,
                            Err(e) => {
                                warn!("pulse encode event: {e}");
                                continue;
                            }
                        };
                        if writer.write_all(&frame).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("pulse client lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn dispatch(
        inner: Arc<DaemonInner>,
        client_id: ClientId,
        request_id: u64,
        req: DaemonReq,
        reply_tx: ReplyTx,
    ) {
        // Read-only commands share a read lock so they are not serialized
        // behind slow mutating commands (Spotify sync, library scan, audio
        // decode, YouTube download). This prevents fast IPC requests such as
        // `get_status` / `ping` from timing out while a long command holds the
        // exclusive lock. Playback transport commands run on their own
        // dedicated lock so they are never queued behind a slow background
        // job either. Network-bound commands (Spotify sync/resolve, YouTube
        // search/download) run on their own serialized lock: they can block a
        // caller for many seconds, but never `GetStatus`/`Ping`.
        let res = if is_read_only(&req) {
            let _guard = inner.cmd_lock.read().await;
            Self::handle_request(&inner, &req, client_id).await
        } else if request_is_playback(&req) {
            let _guard = inner.play_lock.write().await;
            Self::handle_request(&inner, &req, client_id).await
        } else if is_spotify_slow(&req) {
            let _guard = inner.spotify_slow_lock.lock().await;
            Self::handle_request(&inner, &req, client_id).await
        } else if is_yt_slow(&req) {
            let _guard = inner.yt_slow_lock.lock().await;
            Self::handle_request(&inner, &req, client_id).await
        } else {
            let _guard = inner.cmd_lock.write().await;
            Self::handle_request(&inner, &req, client_id).await
        };

        let res = match res {
            Ok(res) => res,
            Err(e) => {
                warn!("command {:?} failed: {e}", req);
                DaemonRes::Error {
                    message: e.to_string(),
                }
            }
        };
        let _ = reply_tx.send((request_id, res));
    }

    async fn handle_request(
        inner: &Arc<DaemonInner>,
        req: &DaemonReq,
        _client_id: ClientId,
    ) -> Result<DaemonRes, CoreError> {
        match req {
            DaemonReq::Play { path, start_pos } => {
                Self::clear_history(inner).await;
                Self::enable_fallback(inner).await;
                Cmd::play(inner, path, *start_pos, false).await
            }
            DaemonReq::PlayStream { url } => {
                Self::clear_history(inner).await;
                Self::enable_fallback(inner).await;
                Cmd::play_url_stream(inner, url).await
            }
            DaemonReq::PlayPause => Cmd::play_pause(inner).await,
            DaemonReq::Pause => Cmd::pause(inner).await,
            DaemonReq::Stop => Cmd::stop(inner).await,
            DaemonReq::Next => Cmd::next(inner).await,
            DaemonReq::Prev => Cmd::prev(inner).await,
            DaemonReq::Seek { position_secs } => Cmd::seek(inner, *position_secs).await,
            DaemonReq::SetVolume { volume } => Cmd::set_volume(inner, *volume).await,
            DaemonReq::GetVolume => Cmd::get_volume(inner).await,
            DaemonReq::ToggleShuffle => Cmd::toggle_shuffle(inner).await,
            DaemonReq::CycleRepeat { mode } => Cmd::set_repeat_mode(inner, *mode).await,
            DaemonReq::ToggleMute => Cmd::toggle_mute(inner).await,
            DaemonReq::SetMono { enabled } => Cmd::set_mono(inner, *enabled).await,
            DaemonReq::Crossfade {
                enabled,
                duration_secs,
            } => Cmd::crossfade(inner, *enabled, *duration_secs).await,
            DaemonReq::SetLoudnessMode { mode } => Cmd::set_loudness_mode(inner, *mode).await,
            DaemonReq::ScanLoudness { track_ids, force } => {
                Cmd::scan_loudness(inner, track_ids.clone(), *force).await
            }
            DaemonReq::SetPreGain { pre_gain_db } => Cmd::set_pre_gain(inner, *pre_gain_db).await,
            DaemonReq::SetGapless { enabled } => Cmd::set_gapless(inner, *enabled).await,
            DaemonReq::SetDynamicMode {
                enabled,
                min_queue_remaining,
                max_history,
            } => Cmd::set_dynamic_mode(inner, *enabled, *min_queue_remaining, *max_history).await,
            DaemonReq::SetScrobble {
                enabled,
                api_key,
                session_token,
                min_play_secs,
                min_play_pct,
            } => {
                Cmd::set_scrobble(
                    inner,
                    *enabled,
                    api_key.clone(),
                    session_token.clone(),
                    *min_play_secs,
                    *min_play_pct,
                )
                .await
            }
            DaemonReq::Library { action } => LibraryHandler::handle(inner, action).await,
            DaemonReq::Search { query } => Search::handle(inner, query).await,
            DaemonReq::GetFavourites => Favourites::list(inner).await,
            DaemonReq::AddFavourite { track_id } => Favourites::add(inner, *track_id).await,
            DaemonReq::RemoveFavourite { track_id } => Favourites::remove(inner, *track_id).await,
            #[cfg(feature = "youtube")]
            DaemonReq::YtSearch { query, filter } => Yt::search(inner, query, *filter).await,
            #[cfg(feature = "youtube")]
            DaemonReq::YtSearchPoll => Yt::poll(inner).await,
            #[cfg(feature = "youtube")]
            DaemonReq::YtSearchCancel => Yt::cancel(inner).await,
            #[cfg(feature = "youtube")]
            DaemonReq::YtResolveStream { url } => Yt::resolve_stream(inner, url).await,
            #[cfg(not(feature = "youtube"))]
            DaemonReq::YtSearch { .. }
            | DaemonReq::YtSearchPoll
            | DaemonReq::YtSearchCancel
            | DaemonReq::YtResolveStream { .. } => Err(CoreError::Daemon(
                "youtube support is disabled in this build".into(),
            )),
            DaemonReq::YtDownload {
                url,
                title,
                channel,
            } => {
                #[cfg(feature = "youtube")]
                {
                    let mut yt = inner.youtube.lock().await;
                    let artist = channel.clone();
                    match yt.download(url.clone(), title.clone(), artist).await {
                        Ok(id) => Ok(DaemonRes::Value {
                            value: serde_json::json!({ "id": id }),
                        }),
                        Err(e) => Err(CoreError::Daemon(e)),
                    }
                }
                #[cfg(not(feature = "youtube"))]
                {
                    Err(CoreError::Daemon(
                        "youtube support is disabled in this build".into(),
                    ))
                }
            }
            DaemonReq::YtDownloadPoll => {
                #[cfg(feature = "youtube")]
                {
                    let mut yt = inner.youtube.lock().await;
                    match yt.poll_download() {
                        Ok(Some(progress)) => {
                            let status_str = format!("{:?}", progress.status).to_lowercase();
                            Ok(DaemonRes::YtDownloadProgress {
                                id: progress.id,
                                url: progress.url,
                                title: progress.title,
                                progress: progress.progress,
                                status: status_str,
                                error: progress.error,
                                file_path: progress.file_path,
                                downloaded_bytes: progress.downloaded_bytes,
                                total_bytes: progress.total_bytes,
                                rate_bps: progress.rate_bps,
                                eta_secs: progress.eta_secs,
                            })
                        }
                        Ok(None) => Ok(DaemonRes::Value {
                            value: serde_json::json!({}),
                        }),
                        Err(e) => Err(CoreError::Daemon(e)),
                    }
                }
                #[cfg(not(feature = "youtube"))]
                {
                    Err(CoreError::Daemon(
                        "youtube support is disabled in this build".into(),
                    ))
                }
            }
            DaemonReq::YtCancelDownload { url: _ } => {
                #[cfg(feature = "youtube")]
                {
                    let mut yt = inner.youtube.lock().await;
                    yt.cancel_download().await;
                    Ok(DaemonRes::Ok)
                }
                #[cfg(not(feature = "youtube"))]
                {
                    Err(CoreError::Daemon(
                        "youtube support is disabled in this build".into(),
                    ))
                }
            }
            DaemonReq::YtFetchPlaylist { url } => {
                #[cfg(feature = "youtube")]
                {
                    let mut yt = inner.youtube.lock().await;
                    match yt.start_fetch_playlist(url.clone()) {
                        Ok(()) => Ok(DaemonRes::Ok),
                        Err(e) => Err(CoreError::Daemon(e)),
                    }
                }
                #[cfg(not(feature = "youtube"))]
                {
                    Err(CoreError::Daemon(
                        "youtube support is disabled in this build".into(),
                    ))
                }
            }
            DaemonReq::YtFetchPlaylistPoll => {
                #[cfg(feature = "youtube")]
                {
                    let mut yt = inner.youtube.lock().await;
                    match yt.poll_playlist() {
                        Ok(Some((query, results))) => {
                            Ok(DaemonRes::YtSearchResults { query, results })
                        }
                        Ok(None) => Ok(DaemonRes::Ok),
                        Err(e) => Err(CoreError::Daemon(e)),
                    }
                }
                #[cfg(not(feature = "youtube"))]
                {
                    Err(CoreError::Daemon(
                        "youtube support is disabled in this build".into(),
                    ))
                }
            }
            #[cfg(feature = "youtube")]
            DaemonReq::YtSetConfig {
                cookie_source,
                cookie_file,
                js_runtime,
                download_dir,
                max_concurrent,
            } => {
                let mut yt = inner.youtube.lock().await;
                yt.set_cookie_file(cookie_file.clone());
                yt.set_cookie_source(cookie_source.clone());
                yt.set_js_runtime(js_runtime.clone());
                yt.set_download_dir(download_dir.clone());
                if let Some(mc) = max_concurrent {
                    yt.set_max_downloads(*mc as usize);
                }
                drop(yt);
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            #[cfg(not(feature = "youtube"))]
            DaemonReq::YtSetConfig { .. } => Ok(DaemonRes::Ok),
            DaemonReq::GetCoverArt { track_id, path } => {
                Cover::get(inner, *track_id, path.clone()).await
            }
            DaemonReq::GetArtistCoverArt { artist } => Cover::artist(inner, artist).await,
            DaemonReq::SetCoverProvider { provider } => {
                *inner.cover_provider_override.lock().await =
                    Some(CoverProvider::from_str_lossy(provider));
                Ok(DaemonRes::Ok)
            }
            DaemonReq::SetCoverCache { bytes } => {
                let cache = inner.cover_cache().await;
                if let Some(cc) = cache.as_ref() {
                    cc.set_disk_cap(*bytes);
                    cc.prune_disk_cache().await;
                }
                Ok(DaemonRes::Ok)
            }
            DaemonReq::GetCoverCacheStat => {
                let (disk_bytes, mem_bytes, cap_bytes) = {
                    let cache = inner.cover_cache().await;
                    match cache.as_ref() {
                        Some(cc) => (cc.disk_use().await, cc.memory_use() as u64, cc.disk_cap()),
                        None => (0, 0, inner.config.cover_cache_bytes),
                    }
                };
                Ok(DaemonRes::CoverCacheStat {
                    disk_bytes,
                    mem_bytes,
                    cap_bytes,
                })
            }
            DaemonReq::GetLyrics { track_id, path } => {
                Lyrics::get(inner, *track_id, path.clone()).await
            }
            DaemonReq::LyricsSearch { artist, title } => Lyrics::search(inner, artist, title).await,
            DaemonReq::SpotifySetToken { token } => Spotify::set_token(inner, token).await,
            DaemonReq::SpotifyOauthStart { client_id, port } => {
                Spotify::oauth_start(inner, client_id, *port).await
            }
            DaemonReq::SpotifyCancelOauth => Spotify::oauth_cancel(inner).await,
            DaemonReq::SpotifyClear => Spotify::clear(inner).await,
            DaemonReq::SpotifyStatus => Spotify::status(inner).await,
            DaemonReq::SpotifyPlayPause => Spotify::play_pause(inner).await,
            DaemonReq::SpotifyNext => Spotify::connect_ctrl(inner, ConnectCmd::Next).await,
            DaemonReq::SpotifyPrevious => Spotify::connect_ctrl(inner, ConnectCmd::Previous).await,
            DaemonReq::SpotifySeek { pos_secs } => {
                Spotify::connect_ctrl(inner, ConnectCmd::Seek(*pos_secs)).await
            }
            DaemonReq::SpotifyShuffle { on } => {
                Spotify::connect_ctrl(inner, ConnectCmd::Shuffle(*on)).await
            }
            DaemonReq::SpotifyRepeat { mode } => {
                Spotify::connect_ctrl(inner, ConnectCmd::Repeat(mode.clone())).await
            }
            DaemonReq::SpotifyVolume { percent } => {
                Spotify::connect_ctrl(inner, ConnectCmd::Volume(*percent)).await
            }
            DaemonReq::SpotifySync => Spotify::sync(inner).await,
            DaemonReq::SpotifyPlaylists => Spotify::playlists(inner).await,
            DaemonReq::SpotifyPlaylistTracks { id } => Spotify::playlist_tracks(inner, id).await,
            DaemonReq::SpotifyResolve {
                playlist_id,
                track_index,
                play,
            } => Spotify::resolve(inner, playlist_id, *track_index, *play).await,
            DaemonReq::SpotifySearchWeb { query } => Spotify::search_web(inner, query).await,
            DaemonReq::SpotifyAlbumTracks { uri } => Spotify::album_tracks(inner, uri).await,
            DaemonReq::SpotifyArtistTopTracks { uri } => {
                Spotify::artist_top_tracks(inner, uri).await
            }
            DaemonReq::SpotifyWebPlaylistTracks { uri } => {
                Spotify::web_playlist_tracks(inner, uri).await
            }
            DaemonReq::SpotifyResolveTrack {
                name,
                artists,
                album,
                uri,
                play,
            } => Spotify::resolve_track(inner, name, artists, album, uri, *play).await,
            DaemonReq::SpotifyPlayAll {
                playlist_id,
                shuffle,
            } => Spotify::play_all(inner, playlist_id, *shuffle).await,
            DaemonReq::SpotifyTrackImage { image_url } => {
                Spotify::track_image(inner, image_url).await
            }
            DaemonReq::ChartsSources => Charts::sources(inner).await,
            DaemonReq::ChartsList { source_id } => Charts::list(inner, source_id.clone()).await,
            DaemonReq::ChartsTracks {
                source_id,
                chart_id,
            } => Charts::tracks(inner, source_id.clone(), chart_id.clone()).await,
            DaemonReq::LastfmSetConfig {
                enabled,
                api_key,
                api_secret,
                session_key,
                min_play_secs,
                min_play_pct,
            } => {
                Lastfm::set_config(
                    inner,
                    *enabled,
                    api_key.clone(),
                    api_secret.clone(),
                    session_key.clone(),
                    *min_play_secs,
                    *min_play_pct,
                )
                .await
            }
            DaemonReq::LastfmAuthUrl => Lastfm::auth_url(inner).await,
            DaemonReq::LastfmOauthStart { port } => Lastfm::oauth_start(inner, *port).await,
            DaemonReq::LastfmAuthenticate { token } => Lastfm::authenticate(inner, token).await,
            DaemonReq::LastfmStatus => Lastfm::status(inner).await,
            DaemonReq::LastfmClear => Lastfm::clear(inner).await,
            DaemonReq::LastfmLove => Lastfm::love(inner).await,
            DaemonReq::LastfmUnlove => Lastfm::unlove(inner).await,
            DaemonReq::PodcastAddFeed { url } => Podcast::add_feed(inner, url).await,
            DaemonReq::PodcastRemoveFeed { feed_id } => Podcast::remove_feed(inner, feed_id).await,
            DaemonReq::PodcastFeeds => Podcast::feeds(inner).await,
            DaemonReq::PodcastEpisodes { feed_id } => Podcast::episodes(inner, feed_id).await,
            DaemonReq::PodcastRefresh { feed_id } => {
                Podcast::refresh(inner, feed_id.as_deref()).await
            }
            DaemonReq::PodcastStatus => Podcast::status(inner).await,
            DaemonReq::PodcastPlay {
                feed_id,
                episode_index,
            } => Podcast::play(inner, feed_id, *episode_index).await,
            DaemonReq::RadioSearch { query, limit } => Radio::search(inner, query, *limit).await,
            DaemonReq::RadioTop { limit } => Radio::top(inner, *limit).await,
            DaemonReq::RadioPlay {
                station_id,
                station_name,
            } => Radio::play(inner, station_id, station_name).await,
            DaemonReq::RadioTags { limit } => Radio::tags(inner, *limit).await,
            DaemonReq::RadioByTag { tag, limit } => Radio::by_tag(inner, tag, *limit).await,
            DaemonReq::RadioCountries { limit } => Radio::countries(inner, *limit).await,
            DaemonReq::RadioByCountry { country, limit } => {
                Radio::by_country(inner, country, *limit).await
            }
            DaemonReq::SetSleepTimer {
                minutes,
                stop_immediately,
            } => Cmd::set_sleep_timer(inner, *minutes, *stop_immediately).await,
            DaemonReq::CancelSleepTimer => Cmd::cancel_sleep_timer(inner).await,
            DaemonReq::SetLowPower { enabled } => Cmd::set_low_power(inner, *enabled).await,
            DaemonReq::GetLowPower => Cmd::get_low_power(inner).await,
            DaemonReq::ListAudioDevices => Cmd::list_audio_devices(inner).await,
            DaemonReq::SetAudioDevice { name } => Cmd::set_audio_device(inner, name.clone()).await,
            DaemonReq::ClearCache { what } => Cmd::clear_cache(inner, *what).await,
            DaemonReq::GetStatus => Cmd::get_status(inner).await,
            DaemonReq::GetStatusLite => Cmd::get_status_lite(inner).await,
            DaemonReq::CheckHealth => Cmd::check_health(inner).await,
            DaemonReq::Ping => Ok(DaemonRes::Pong),
            DaemonReq::Queue { action } => Queue::handle(inner, action).await,
            DaemonReq::SetEqPreset { preset } => Cmd::set_eq_preset(inner, *preset).await,
            DaemonReq::SetEqEnabled { enabled } => Cmd::set_eq_enabled(inner, *enabled).await,
            DaemonReq::SetReverb { enabled, room_size } => {
                Cmd::set_reverb(inner, *enabled, *room_size).await
            }
            DaemonReq::ListEqPresets => Cmd::list_eq_presets(inner).await,
            DaemonReq::SetSpeed { rate } => Cmd::set_speed(inner, *rate).await,
            DaemonReq::GetSpeed => Cmd::get_speed(inner).await,
            DaemonReq::Quit => {
                info!("quit requested");
                // Capture the full state (including the live current track and
                // position) *before* stopping so the next start can resume
                // exactly as the user left it — `Cmd::stop` clears those fields.
                let cleanup_state = if !inner.config.test_mode {
                    let s = inner.state.read().await;
                    Some((SavedState::from_state(&s), inner.config.state_file.clone()))
                } else {
                    None
                };
                let _ = Cmd::stop(inner).await;
                let _ = inner.event_tx.send(DaemonEvent::Custom {
                    name: "daemon_quitting".into(),
                    data: [].into(),
                });
                let socket_path = inner.config.socket_path.clone();
                let socket_pulse_path = inner.config.socket_pulse_path.clone();
                tokio::spawn(async move {
                    // Perform the blocking state save and file removal off the
                    // async thread so shutdown never stalls the event loop.
                    if let Some((saved, state_file)) = cleanup_state {
                        let _ = tokio::task::spawn_blocking(move || saved.save(&state_file))
                            .await
                            .map_err(|e| warn!("state save join failed: {e}"))
                            .map(|r| {
                                if let Err(e) = r {
                                    warn!("failed to save state on quit: {e}");
                                }
                            });
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let _ = std::fs::remove_file(&socket_path);
                    let _ = std::fs::remove_file(&socket_pulse_path);
                    let _ = std::fs::remove_file(resolve_pid_file());
                    info!("daemon shut down cleanly");
                    std::process::exit(0);
                });
                Ok(DaemonRes::Ok)
            }
        }
    }

    fn push_event(inner: &DaemonInner, event: DaemonEvent) {
        let _ = inner.event_tx.send(event);
    }

    fn save_state(inner: &DaemonInner) {
        if inner.config.test_mode {
            return;
        }
        let state_file = inner.config.state_file.clone();
        let state = inner.state.clone();
        let clients = inner.active_clients.load(Ordering::Relaxed);
        tokio::spawn(async move {
            // Skip the full queue clone + JSON serialization when the daemon
            // is truly idle: no clients connected, playback stopped at
            // position 0, and an empty queue. Nothing has changed to persist
            // since the last save, so this avoids re-encoding the same state
            // every minute.
            if clients == 0 {
                let s = state.read().await;
                let idle =
                    s.status == PlaybackStatus::Stopped && s.time_pos <= 0.0 && s.queue.is_empty();
                drop(s);
                if idle {
                    return;
                }
            }
            let s = state.read().await;
            let saved = SavedState::from_state(&s);
            drop(s);
            if let Err(e) = saved.save(&state_file) {
                warn!("failed to save state: {e}");
            }
        });
    }

    /// Release everything only a connected client would use once the last
    /// client disconnects (and there is nothing playing headlessly):
    ///
    /// - the auto-advance library fallback list and radio rotation ring;
    /// - the in-memory play history (bounded anyway, but dropped wholesale);
    /// - stale spectrum levels;
    /// - the cover-art LRU caches and lyrics manager with their TLS pools,
    ///   rebuilt lazily on the next request.
    ///
    /// Playback state (queue, current track, position) is preserved so a still
    /// playing stream keeps auto-advancing exactly as before.
    async fn maybe_prune_idle(inner: &DaemonInner) {
        {
            let mut state = inner.state.write().await;
            state.audio_levels.clear();
            if state.status == PlaybackStatus::Stopped {
                state.default_list.clear();
                state.default_cursor = 0;
                state.radio_history.clear();
            }
        }
        inner.play_history.lock().await.clear();
        *inner.cover_cache.lock().await = None;
        *inner.lyrics_manager.lock().await = None;
        info!("pruned idle state (no clients connected)");
    }

    /// Persist a listen to the SQLite library: increments `play_count` and
    /// stamps `last_played`. Runs off-thread; failures are logged, never
    /// fatal. Tracks with no library row (streams, radio, remote Spotify)
    /// simply match zero rows.
    async fn record_play(inner: &DaemonInner, track: &TrackInfo) {
        if track.id <= 0 {
            return;
        }
        let data_dir = inner.config.data_dir.clone();
        let id = track.id;
        tokio::task::spawn_blocking(move || {
            let Ok(lib) = Library::new(data_dir.to_str().unwrap_or("")) else {
                return;
            };
            if let Err(e) = lib.record_play(id) {
                tracing::warn!("record play metrics: {e}");
            }
        });
    }

    async fn enable_fallback(inner: &DaemonInner) {
        inner.state.write().await.fallback_disabled = false;
    }

    async fn clear_history(inner: &DaemonInner) {
        inner.play_history.lock().await.clear();
    }

    async fn push_queue_state(inner: &DaemonInner) {
        let state = inner.state.read().await;
        let (queue, cursor) = queue::visible(&state);
        drop(state);
        Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
    }

    fn next_track(state: &DaemonState) -> Option<TrackInfo> {
        let cur_is_queued = state
            .current_track
            .as_ref()
            .map(|t| {
                state
                    .queue
                    .first()
                    .map(|q| q.path == t.path)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !state.queue.is_empty() {
            if cur_is_queued {
                state
                    .queue
                    .get(1)
                    .cloned()
                    .or_else(|| state.default_list.get(state.default_cursor + 1).cloned())
            } else {
                state.queue.first().cloned()
            }
        } else if !state.default_list.is_empty() {
            let len = state.default_list.len();
            let cursor = state.default_cursor.min(len - 1);
            if cursor + 1 < len {
                state.default_list.get(cursor + 1).cloned()
            } else {
                match state.repeat {
                    RepeatMode::All => state.default_list.first().cloned(),
                    _ => None,
                }
            }
        } else {
            None
        }
    }

    async fn build_default_list(inner: &DaemonInner, resume_key: Option<&str>) -> Vec<TrackInfo> {
        let data_dir = inner.config.data_dir.clone();
        let tracks = tokio::task::spawn_blocking(move || {
            Library::new(data_dir.to_str().unwrap_or(""))
                .ok()
                .and_then(|lib| lib.list_tracks().ok())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        if tracks.is_empty() {
            return tracks;
        }
        let mut list = tracks;
        list.sort_by_key(|a| a.title.to_lowercase());
        let shuffle = inner.state.read().await.shuffle;
        if shuffle {
            fastrand::shuffle(&mut list);
        } else if let Some(key) = resume_key {
            let key_char = key
                .chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .unwrap_or('\0');
            let pos = list.iter().position(|t| {
                t.title
                    .chars()
                    .next()
                    .map(|c| c.to_lowercase().next().unwrap_or(c))
                    .unwrap_or('\0')
                    >= key_char
            });
            if let Some(pos) = pos
                && pos > 0
            {
                list.rotate_left(pos);
            }
        }
        list
    }

    async fn step_next(inner: &DaemonInner) -> Result<Option<TrackInfo>, CoreError> {
        let mut resume_key: Option<String> = None;
        {
            let mut history = inner.play_history.lock().await;
            let mut state = inner.state.write().await;
            if !state.queue.is_empty() {
                let cur_is_queued = state
                    .current_track
                    .as_ref()
                    .map(|t| t.path == state.queue[0].path)
                    .unwrap_or(false);
                if cur_is_queued {
                    let cur = state.queue.remove(0);
                    resume_key = Some(cur.title.clone());
                    if cur.path.starts_with("radio://") {
                        state.radio_history.push(cur.clone());
                        if state.radio_history.len() > MAX_RADIO_HISTORY {
                            state.radio_history.remove(0);
                        }
                    }
                    history.push(HistoryEntry::User(cur));
                    if history.len() > MAX_HISTORY {
                        history.remove(0);
                    }
                    if !state.queue.is_empty() {
                        let next = state.queue[0].clone();
                        drop(state);
                        drop(history);
                        return Ok(Some(next));
                    }
                } else {
                    let next = state.queue[0].clone();
                    drop(state);
                    drop(history);
                    return Ok(Some(next));
                }
            }
            // If the current track was a radio station, cycle through radio history
            if state
                .current_track
                .as_ref()
                .is_some_and(|t| t.path.starts_with("radio://"))
                && !state.radio_history.is_empty()
            {
                let next = state.radio_history.remove(0);
                drop(state);
                drop(history);
                return Ok(Some(next));
            }
            if !state.default_list.is_empty() {
                let len = state.default_list.len();
                let cursor = state.default_cursor;
                if cursor < len {
                    let cur = state.default_list[cursor].clone();
                    history.push(HistoryEntry::Default {
                        index: cursor,
                        track: cur.clone(),
                    });
                    if history.len() > MAX_HISTORY {
                        history.remove(0);
                    }
                    let next_idx = cursor + 1;
                    if next_idx < len {
                        state.default_cursor = next_idx;
                        let next = state.default_list[next_idx].clone();
                        drop(state);
                        drop(history);
                        return Ok(Some(next));
                    }
                    state.default_cursor = next_idx;
                    let next = match state.repeat {
                        RepeatMode::Off => None,
                        RepeatMode::All => {
                            state.default_cursor = 0;
                            state.default_list.first().cloned()
                        }
                        RepeatMode::One => {
                            state.default_cursor = cursor;
                            Some(cur)
                        }
                    };
                    drop(state);
                    drop(history);
                    return Ok(next);
                } else {
                    let next = match state.repeat {
                        RepeatMode::Off => None,
                        RepeatMode::All => {
                            state.default_cursor = 0;
                            state.default_list.first().cloned()
                        }
                        RepeatMode::One => {
                            state.default_cursor = len - 1;
                            state.default_list.get(len - 1).cloned()
                        }
                    };
                    drop(state);
                    drop(history);
                    return Ok(next);
                }
            }
            if resume_key.is_none() {
                resume_key = state.current_track.as_ref().map(|t| t.title.clone());
            }
            let fallback_disabled = state.fallback_disabled;
            drop(state);
            drop(history);
            if fallback_disabled {
                return Ok(None);
            }
        }
        let list = Self::build_default_list(inner, resume_key.as_deref()).await;
        if list.is_empty() {
            return Ok(None);
        }
        let first = list[0].clone();
        {
            let mut state = inner.state.write().await;
            state.default_list = list;
            state.default_cursor = 0;
            state.fallback_disabled = false;
        }
        Ok(Some(first))
    }

    /// Re-play a live `radio://` path after a dropped connection, with capped
    /// backoff. Aborts when `play_session` changes (user stop/next/prev/play)
    /// or the current track moved on, so retries never fight user input. Falls
    /// back to `stop_playback` only after repeated failures.
    async fn retry_live_stream(inner: &Arc<DaemonInner>, path: &str, session: u64) {
        let mut delay_secs = 2u64;
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            if inner.play_session.load(Ordering::Acquire) != session {
                return;
            }
            let still_current = {
                let state = inner.state.read().await;
                state.current_track.as_ref().is_some_and(|t| t.path == path)
            };
            if !still_current {
                return;
            }
            // `Cmd::play` bumps the session itself; a concurrent user action
            // racing us just wins and our next guard bails out.
            let _lock = inner.play_lock.write().await;
            if inner.play_session.load(Ordering::Acquire) != session {
                return;
            }
            match Cmd::play(inner, path, 0.0, false).await {
                Ok(_) => return,
                Err(e) => warn!("radio reconnect for {path} failed: {e}"),
            }
            delay_secs = (delay_secs * 2).min(30);
        }
        if inner.play_session.load(Ordering::Acquire) == session {
            warn!("radio reconnect for {path} gave up after retries");
            Self::stop_playback(inner).await;
        }
    }

    async fn stop_playback(inner: &DaemonInner) {
        inner.play_session.fetch_add(1, Ordering::Release);
        // A pending stop-at-track-end sleep timer no longer applies once the
        // user has explicitly stopped playback.
        inner.sleep_stop_at_track_end.store(false, Ordering::SeqCst);
        {
            let mut mixer = inner.mixer.lock().await;
            let _ = mixer.stop();
        }
        *inner.crossfade_loaded_for.lock().await = None;
        *inner.countdown_notified_for.lock().await = None;

        // Scrobble current track if it was played long enough
        let (track, played_secs) = {
            let state = inner.state.read().await;
            (state.current_track.clone(), state.time_pos.max(0.0))
        };
        if let Some(track) = track {
            Cmd::scrobble_track(inner, &track, played_secs).await;
        }

        let mut state = inner.state.write().await;
        state.status = PlaybackStatus::Stopped;
        state.current_track = None;
        state.time_pos = 0.0;
        inner.scrobble.lock().await.start("", 0.0);
        drop(state);
        Self::push_event(inner, DaemonEvent::TrackEnded);
    }

    /// Shared sleep-timer expiry shutdown: silence the mixer and any Web
    /// (Spotify/yt-dlp) stream, reset the transport, and report the state
    /// change. Used by the immediate expiry path and by the deferred
    /// stop-at-track-end path once the current finite track finishes.
    async fn sleep_timer_expiry_stop(inner: &DaemonInner) {
        {
            let mut mixer = inner.mixer.lock().await;
            let _ = mixer.stop();
            let speed = inner.state.read().await.audio.speed;
            mixer.set_speed(speed);
        }
        inner.stream.lock().await.reset();
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut s = inner.state.write().await;
            s.status = PlaybackStatus::Stopped;
            s.sleep_timer = None;
            s.version += 1;
        }
        Self::push_event(inner, DaemonEvent::PlaybackStopped);
        Self::push_event(inner, DaemonEvent::SleepTimerExpired);
    }

    /// Mirrors the client's `is_live_stream`: `radio://` stations and bare
    /// `http(s)://` stream URLs (LoadStream) have no defined track end, so a
    /// deferred (stop-at-end-of-track) sleep timer must not wait for one.
    fn path_is_live_stream(path: &str) -> bool {
        if path.starts_with("radio://") {
            return true;
        }
        if path.starts_with("http://") || path.starts_with("https://") {
            return !is_youtube(path);
        }
        false
    }

    async fn try_start_crossfade(inner: &DaemonInner, track: &TrackInfo) -> bool {
        let session = inner.play_session.load(Ordering::Acquire);
        let (enabled, dur) = {
            let state = inner.state.read().await;
            match state.crossfade.as_ref() {
                Some(cf) => (cf.enabled, cf.duration_secs as f64),
                None => (false, 0.0),
            }
        };
        if !enabled
            || dur <= 0.0
            || inner.crossfade_loaded_for.lock().await.is_some()
            || inner.mixer.lock().await.is_crossfading()
        {
            return false;
        }
        let path = track.path.clone();
        // librespot keeps a single player per session, and `StreamManager::load`
        // tears the current one down, so a spotify: URI cannot be pre-decoded
        // for standby without dropping the playing track. Crossfade is skipped
        // for it and the normal advance path plays it instead.
        if path.starts_with("spotify:") {
            return false;
        }
        let is_remote = parse_remote_path(&path).is_some();
        let decoded = if is_remote {
            match resolve_remote(inner, &path).await {
                Ok(resolved) => {
                    let (url, live) = (resolved.url, resolved.live);
                    let start = 0.0;
                    tokio::task::spawn_blocking(move || {
                        decode_remote_reader(url, live, start, None)
                    })
                    .await
                }
                Err(e) => {
                    warn!("crossfade resolve failed: {e}");
                    return false;
                }
            }
        } else {
            let path_owned = path.clone();
            tokio::task::spawn_blocking(move || AudioMixer::decode_file(&path_owned)).await
        };
        let source = match decoded {
            Ok(Ok(source)) => source,
            _ => return false,
        };
        {
            let mut mixer = inner.mixer.lock().await;
            // A user play/stop was issued while the next track was decoding:
            // abandon the stale load instead of crossfading over it.
            if inner.play_session.load(Ordering::Acquire) != session {
                return false;
            }
            if mixer.load_standby_decoded(source).is_err() {
                return false;
            }
            mixer.start_crossfade(dur);
        }
        // Re-verify after the mixer mutation: a play/stop that ran while we
        // held the lock would have swapped the session, so the marker below
        // must not be published against a fresh playback state.
        if inner.play_session.load(Ordering::Acquire) != session {
            return false;
        }
        *inner.crossfade_loaded_for.lock().await = Some(track.path.clone());
        true
    }

    /// Resolve a local file's metadata (library lookup, then tag read) without
    /// blocking the async worker thread: the SQLite open and lofty tag parse
    /// run inside `spawn_blocking`. Returns the tag-derived `TrackInfo`, or a
    /// filename-derived fallback if the metadata gather panics.
    async fn resolve_track_meta(
        inner: &DaemonInner,
        path: &std::path::Path,
        dur: f64,
    ) -> TrackInfo {
        let ctx = MetaCtx {
            data_dir: inner.config.data_dir.to_string_lossy().into_owned(),
            cache_dir: inner.config.cache_dir.to_string_lossy().into_owned(),
            test_mode: inner.config.test_mode,
        };
        let path = path.to_path_buf();
        let path_for_blocking = path.clone();
        tokio::task::spawn_blocking(move || resolve_meta_sync(&ctx, &path_for_blocking, dur))
            .await
            .unwrap_or_else(|_| {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let (cleaned_artist, cleaned_title) = clean_filename_stem(&stem);
                TrackInfo {
                    id: 0,
                    path: path.to_string_lossy().into_owned(),
                    title: if cleaned_title.is_empty() {
                        stem
                    } else {
                        cleaned_title
                    },
                    artist: cleaned_artist.unwrap_or_else(|| "Unknown Artist".to_string()),
                    album: "Unknown Album".to_string(),
                    duration: dur,
                    ..Default::default()
                }
            })
    }

    async fn finish_crossfade(inner: &DaemonInner) {
        let actual = inner.mixer.lock().await.current_position();
        *inner.crossfade_loaded_for.lock().await = None;
        *inner.countdown_notified_for.lock().await = None;

        // Scrobble the track that just finished before advancing.
        let prev_track = {
            let state = inner.state.read().await;
            state.current_track.clone()
        };
        if let Some(track) = prev_track {
            Cmd::scrobble_track(inner, &track, inner.state.read().await.time_pos).await;
        }

        match Self::step_next(inner).await {
            Ok(Some(mut next)) => {
                let dur = inner.mixer.lock().await.duration();
                // Remote queue entries keep their client-supplied metadata
                // (provider name/album); only local files are re-metadata'd.
                if parse_remote_path(&next.path).is_none() && !next.path.starts_with("spotify:") {
                    next = Self::resolve_track_meta(inner, std::path::Path::new(&next.path), dur)
                        .await;
                } else if dur > 0.0 {
                    next.duration = dur;
                }
                {
                    let mut state = inner.state.write().await;
                    state.status = PlaybackStatus::Playing;
                    state.time_pos = actual;
                    state.current_track = Some(next.clone());
                    state.duration = dur;
                }
                inner.scrobble.lock().await.start(&next.path, actual);
                // Update Last.fm now playing for the new track
                if let Some(ref track) = inner.state.read().await.current_track {
                    let lastfm = inner.lastfm.lock().await;
                    if lastfm.is_ready().await {
                        let _ = tokio::time::timeout(
                            Duration::from_secs(10),
                            lastfm.update_now_playing(track),
                        )
                        .await;
                    }
                }
                Self::push_event(
                    inner,
                    DaemonEvent::PlaybackStarted {
                        track: next,
                        auto_advanced: true,
                        time_pos: actual,
                        duration: dur,
                    },
                );
                Self::push_queue_state(inner).await;
            }
            Ok(None) => {
                Self::stop_playback(inner).await;
            }
            Err(e) => {
                warn!("crossfade finish failed: {e}");
                Self::stop_playback(inner).await;
            }
        }
    }

    /// Mirror the active live stream's ICY `StreamTitle` into `state` and
    /// broadcast a `RadioTitleChanged` event when it changes. Non-live
    /// playback clears any stale title so it never leaks onto a local track.
    /// Called from the ~1 Hz position tick.
    async fn sync_radio_title(inner: &Arc<DaemonInner>) {
        let slot_title = inner.icy_title.lock().unwrap().clone();
        let mut state = inner.state.write().await;
        let is_live = state
            .current_track
            .as_ref()
            .map(|t| {
                t.path.starts_with("radio://")
                    || t.path.starts_with("http://")
                    || t.path.starts_with("https://")
            })
            .unwrap_or(false);
        if is_live {
            if state.radio_title.as_deref() == slot_title.as_deref() {
                return;
            }
            state.radio_title = slot_title;
        } else if state.radio_title.is_none() {
            return;
        } else {
            state.radio_title = None;
        }
        let title = state.radio_title.clone();
        drop(state);
        Self::push_event(inner, DaemonEvent::RadioTitleChanged { title });
    }

    async fn handle_audio_event(inner: &Arc<DaemonInner>, result: AudioResult<Option<AudioEvent>>) {
        let ev = match result {
            Ok(Some(e)) => e,
            Ok(None) => {
                return;
            }
            Err(e) => {
                warn!("backend error: {e}");
                Self::push_event(
                    inner,
                    DaemonEvent::Custom {
                        name: "backend_error".into(),
                        data: [("error".into(), e.to_string())].into(),
                    },
                );
                return;
            }
        };

        match ev {
            AudioEvent::Position(pos) => {
                // A deferred (stop-at-track-end) sleep timer is armed and the
                // current finite track is within the crossfade window: promotion
                // would revive playback via the standby source, defeating the
                // deferral. Stop instead — the track's natural end is imminent.
                if inner.sleep_stop_at_track_end.load(Ordering::SeqCst)
                    && inner.crossfade_loaded_for.lock().await.is_some()
                {
                    Daemon::sleep_timer_expiry_stop(inner).await;
                    return;
                }
                if inner.crossfade_loaded_for.lock().await.is_some()
                    && !inner.mixer.lock().await.is_crossfading()
                {
                    // Finish the crossfade on a detached task so the 16ms poll
                    // loop does not stall waiting on the exclusive lock while a
                    // slow mutating command holds it.
                    let inner = Arc::clone(inner);
                    let session = inner.play_session.load(Ordering::Acquire);
                    tokio::spawn(async move {
                        let _lock = inner.play_lock.write().await;
                        // A play/stop raced the task start: the user switched
                        // sources, so this auto-advance must not run.
                        if inner.play_session.load(Ordering::Acquire) != session {
                            return;
                        }
                        Self::finish_crossfade(&inner).await;
                    });
                }
                let mut state = inner.state.write().await;
                state.time_pos = pos;
                let dur = state.duration;
                let crossfade = state.crossfade.clone();
                let next = Self::next_track(&state);

                // Accrue genuinely-played time for the scrobble threshold.
                let tracking = (
                    state.current_track.as_ref().map(|t| t.path.clone()),
                    state.status == PlaybackStatus::Playing,
                );
                drop(state);
                if tracking.1
                    && let Some(key) = tracking.0
                {
                    inner.scrobble.lock().await.tick(&key, pos);
                }

                // Re-anchor client clocks at ~1 Hz so the TUI's extrapolated
                // position (and hence the lyric highlight) never drifts by
                // more than a fraction of a second. Seek and track-change
                // broadcasts still happen immediately on their own paths.
                {
                    let mut last = inner.last_pos_broadcast.lock().await;
                    let due = last
                        .as_ref()
                        .map(|t| t.elapsed() >= std::time::Duration::from_secs(1))
                        .unwrap_or(true);
                    if due {
                        *last = Some(std::time::Instant::now());
                        Self::push_event(inner, DaemonEvent::PositionChanged { time_pos: pos });
                        Self::sync_radio_title(inner).await;
                    }
                }

                let cf_secs = crossfade
                    .as_ref()
                    .filter(|c| c.enabled)
                    .map_or(0.0, |c| c.duration_secs as f64);
                if dur > 0.0
                    && (dur - pos) <= cf_secs + 3.0
                    && let Some(track) = &next
                {
                    let mut notified = inner.countdown_notified_for.lock().await;
                    if notified.as_deref() != Some(track.hash.as_str()) {
                        *notified = Some(track.hash.clone());
                        Self::push_event(
                            inner,
                            DaemonEvent::CrossfadeCountdown {
                                track: track.clone(),
                            },
                        );
                    }
                }

                if let Some(cf) = crossfade
                    && cf.enabled
                    && dur > 0.0
                    && (dur - pos) <= cf.duration_secs as f64 + 0.15
                    && let Some(track) = &next
                    && !inner.sleep_stop_at_track_end.load(Ordering::SeqCst)
                {
                    let _ = Self::try_start_crossfade(inner, track).await;
                }
            }
            AudioEvent::Duration(dur) => {
                let mut state = inner.state.write().await;
                state.duration = dur;
                drop(state);
                Self::push_event(inner, DaemonEvent::DurationChanged { duration: dur });
            }
            AudioEvent::Finished => {
                // A dropped live stream reconnects, it doesn't stop: `radio://`
                // streams are endless, so `Finished` only fires when the server
                // closed the connection or the transport hit EOF. The daemon
                // re-plays the same station path (which re-resolves the stream
                // URL, since those expire) with backoff; an explicit user
                // stop/next bumps `play_session`, which aborts the retries.
                // Manual Next keeps working (it switches the source before any
                // `Finished` can fire).
                let radio_path = {
                    let state = inner.state.read().await;
                    state
                        .current_track
                        .as_ref()
                        .filter(|t| t.path.starts_with("radio://"))
                        .map(|t| t.path.clone())
                };
                if let Some(path) = radio_path {
                    let inner = Arc::clone(inner);
                    let session = inner.play_session.load(Ordering::Acquire);
                    tokio::spawn(async move {
                        Self::retry_live_stream(&inner, &path, session).await;
                    });
                    return;
                }
                // Deferred sleep timer: the current finite track just ended
                // naturally, so stop instead of advancing to the next track.
                if inner.sleep_stop_at_track_end.swap(false, Ordering::SeqCst) {
                    Self::sleep_timer_expiry_stop(inner).await;
                    return;
                }
                let was_crossfading = inner.crossfade_loaded_for.lock().await.is_some();
                if was_crossfading {
                    // Run on a detached task to avoid blocking the poll loop on
                    // the exclusive lock during next-track startup.
                    let inner = Arc::clone(inner);
                    let session = inner.play_session.load(Ordering::Acquire);
                    tokio::spawn(async move {
                        let _lock = inner.play_lock.write().await;
                        // A play/stop raced the task: don't advance over a
                        // fresh user-initiated playback.
                        if inner.play_session.load(Ordering::Acquire) != session {
                            return;
                        }
                        Self::finish_crossfade(&inner).await;
                        let _ = Cmd::next(&inner).await;
                    });
                } else {
                    let _ = inner.internal_req_tx.send(DaemonReq::Next);
                }
            }
            AudioEvent::Error(msg) => {
                warn!("audio error: {msg}");
                Self::push_event(
                    inner,
                    DaemonEvent::Custom {
                        name: "audio_error".into(),
                        data: [("error".into(), msg)].into(),
                    },
                );
                // Same reconnect policy as `Finished`: a live radio transport
                // error resumes the station instead of going silent.
                let radio_path = {
                    let state = inner.state.read().await;
                    state
                        .current_track
                        .as_ref()
                        .filter(|t| t.path.starts_with("radio://"))
                        .map(|t| t.path.clone())
                };
                if let Some(path) = radio_path {
                    let inner = Arc::clone(inner);
                    let session = inner.play_session.load(Ordering::Acquire);
                    tokio::spawn(async move {
                        Self::retry_live_stream(&inner, &path, session).await;
                    });
                }
            }
        }
    }

    // ─── Command handlers ──────────────────────────────────────────────
    //
    //

    async fn promote_crossfade(inner: &DaemonInner) -> Option<String> {
        let mut mixer = inner.mixer.lock().await;
        if !mixer.is_crossfading() || !mixer.standby_is_loaded() {
            return None;
        }
        mixer.drop_active();
        drop(mixer);
        inner.crossfade_loaded_for.lock().await.take()
    }

    async fn report_promoted(inner: &DaemonInner, path: &str) {
        let dur = inner.mixer.lock().await.duration();
        let track = Self::resolve_track_meta(inner, std::path::Path::new(path), dur).await;
        {
            let mut state = inner.state.write().await;
            state.status = PlaybackStatus::Playing;
            state.time_pos = 0.0;
            state.current_track = Some(track.clone());
            state.duration = dur;
        }
        Self::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced: true,
                time_pos: 0.0,
                duration: dur,
            },
        );
        Self::push_queue_state(inner).await;
    }

    // ─── Spotify ───

    #[cfg(feature = "youtube")]
    async fn download_to_cache(
        cache_dir: &Path,
        prefix: &str,
        url: &str,
        auth: Vec<std::ffi::OsString>,
        sem: std::sync::Arc<tokio::sync::Semaphore>,
        gate: std::sync::Arc<tokio::sync::Mutex<std::time::Instant>>,
    ) -> Result<String, String> {
        let max_retries = 3u32;
        let mut last_err = String::new();
        for attempt in 1..=max_retries {
            match Self::try_cache_download(
                cache_dir,
                prefix,
                url,
                auth.clone(),
                sem.clone(),
                gate.clone(),
            )
            .await
            {
                Ok(path) => return Ok(path),
                Err(e) => {
                    last_err = e;
                    if attempt < max_retries {
                        tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    #[cfg(feature = "youtube")]
    async fn try_cache_download(
        cache_dir: &Path,
        prefix: &str,
        url: &str,
        auth: Vec<std::ffi::OsString>,
        sem: std::sync::Arc<tokio::sync::Semaphore>,
        gate: std::sync::Arc<tokio::sync::Mutex<std::time::Instant>>,
    ) -> Result<String, String> {
        let dir = cache_dir.join("spotify");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("create spotify cache: {e}"))?;
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(prefix) {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
        let path = download_into(url, &dir, prefix, &auth, &sem, &gate).await?;
        Ok(path.to_string_lossy().into_owned())
    }
}

/// View of the daemon config needed to resolve track metadata off the async
/// runtime; owned so it can be moved into `spawn_blocking`.
struct MetaCtx {
    data_dir: String,
    cache_dir: String,
    test_mode: bool,
}

/// Blocking metadata resolution for a local file: SQLite library lookup
/// followed by a lofty tag read, then a filename-stem fallback.
fn resolve_meta_sync(ctx: &MetaCtx, path: &std::path::Path, dur: f64) -> TrackInfo {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path_str = path.to_string_lossy().into_owned();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    if !ctx.test_mode {
        if let Ok(lib) = Library::new(&ctx.data_dir) {
            if let Ok(Some(mut t)) = lib.track_by_path(&path_str) {
                t.duration = dur;
                return t;
            }
            if let Ok(tracks) = lib.list_tracks()
                && let Some(matched) = tracks
                    .iter()
                    .find(|t| path_str.contains(&t.path) || t.path.contains(&path_str))
            {
                let mut t = matched.clone();
                t.duration = dur;
                return t;
            }
        }

        if let Ok((meta, hash)) = extract_metadata(&path_str, Some(&ctx.cache_dir)) {
            return TrackInfo {
                id: 0,
                path: path_str,
                title: if meta.title.is_empty() {
                    stem.clone()
                } else {
                    meta.title
                },
                artist: if meta.artist.is_empty() {
                    "Unknown Artist".to_string()
                } else {
                    meta.artist
                },
                album: if meta.album.is_empty() {
                    "Unknown Album".to_string()
                } else {
                    meta.album
                },
                duration: if dur > 0.0 { dur } else { meta.duration },
                track_number: meta.track_number,
                genre: meta.genre,
                year: meta.year,
                bitrate: meta.bitrate,
                samplerate: meta.samplerate,
                hash,
                cover_path: meta.cover_path,
                favourite: false,
                ..Default::default()
            };
        }
    }

    let (cleaned_artist, cleaned_title) = clean_filename_stem(&stem);
    let title = if cleaned_title.is_empty() {
        stem
    } else {
        cleaned_title
    };
    let artist = cleaned_artist.unwrap_or_else(|| "Unknown Artist".to_string());
    TrackInfo {
        id: 0,
        path: path_str,
        title,
        artist,
        album: "Unknown Album".to_string(),
        duration: dur,
        ..Default::default()
    }
}

fn metadata_query_for(track: &TrackInfo) -> (String, String) {
    let stem = Path::new(&track.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (cleaned_artist, cleaned_title) = clean_filename_stem(stem);

    // A freshly YouTube-downloaded track carries the raw video title and the
    // uploader as the artist, neither of which Deezer can match (e.g. artist
    // "DrakeVEVO"). When the title carries YouTube clutter or the artist looks
    // like a channel, query with the cleaned values so the cover art and metadata
    // are fetched immediately after the download.
    let (yt_artist, yt_title) = clean_youtube_title(&track.title);
    let youtube_harvest = yt_title != track.title || is_channel_artist(&track.artist);
    if youtube_harvest {
        let artist = yt_artist
            .or_else(|| cleaned_artist.clone())
            .unwrap_or_default();
        let title = if !yt_title.is_empty() {
            yt_title
        } else {
            cleaned_title
        };
        return (artist, title);
    }

    let title_unreliable = is_filename_like(stem, &track.title);
    let query_artist = if track.artist.is_empty() {
        cleaned_artist.unwrap_or_default()
    } else {
        track.artist.clone()
    };
    let query_title = if title_unreliable {
        cleaned_title
    } else {
        track.title.clone()
    };
    (query_artist, query_title)
}

/// True when the artist looks like a YouTube channel/uploader name rather than
/// a real artist, so the metadata query can prefer the cleaned values.
fn is_channel_artist(artist: &str) -> bool {
    let a = artist.trim();
    a.is_empty() || a.contains("VEVO") || a.contains(" - Topic") || a.contains(" · Topic")
}

fn run_covers_sync(
    data_dir: PathBuf,
    cache_dir: PathBuf,
    provider: CoverProvider,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let mut cache = CoverCache::new(cache_dir.clone());
    let mut synced = 0usize;
    for track in &tracks {
        let missing_cover = track.cover_path.is_none()
            || track
                .cover_path
                .as_ref()
                .is_none_or(|p| !std::path::Path::new(p).exists());
        if !missing_cover {
            continue;
        }
        let artist = if track.artist.is_empty() {
            "Unknown Artist"
        } else {
            &track.artist
        };
        let album = if track.album.is_empty() {
            "Unknown Album"
        } else {
            &track.album
        };
        if rt
            .block_on(tokio::time::timeout(
                std::time::Duration::from_secs(12),
                cache.get(artist, album, provider),
            ))
            .ok()
            .flatten()
            .is_some()
        {
            let key = CoverCache::cache_key(artist, album);
            let cover_file = cache_dir.join("covers").join(format!("{key}.jpg"));
            if cover_file.exists() {
                let path_str = cover_file.to_string_lossy().to_string();
                let _ = lib.update_cover_path(track.id, &path_str);
            }
            synced += 1;
            progress.synced.store(synced, Ordering::Relaxed);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok((synced, total))
}

fn run_lyrics_sync(
    data_dir: PathBuf,
    lyrics_manager: Option<LyricsManager>,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let manager = lyrics_manager.ok_or("lyrics manager not available")?;
    let mut synced = 0usize;
    for track in &tracks {
        let lrc_path = std::path::Path::new(&track.path).with_extension("lrc");
        if lrc_path.exists() {
            continue;
        }
        if let Some(lyrics) = rt
            .block_on(tokio::time::timeout(
                std::time::Duration::from_secs(15),
                manager.get_lyrics(track),
            ))
            .ok()
            .flatten()
            && !lyrics.lines.is_empty()
        {
            let lrc_content = lrc_to_text(&lyrics);
            if std::fs::write(&lrc_path, &lrc_content).is_ok() {
                synced += 1;
                progress.synced.store(synced, Ordering::Relaxed);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok((synced, total))
}

fn run_metadata_sync(
    data_dir: PathBuf,
    cache_dir: PathBuf,
    only_path: Option<String>,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let deezer = DeezerSearch::new();
    let mut synced = 0usize;
    for track in &tracks {
        let stem = Path::new(&track.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if let Some(ref only) = only_path {
            if track.path != *only {
                continue;
            }
        } else if !tags_need_enrichment(
            stem,
            &track.title,
            &track.artist,
            &track.album,
            &track.genre,
            track.track_number.unwrap_or(0),
        ) {
            continue;
        }

        let (q_artist, q_title) = metadata_query_for(track);
        let hit = match rt.block_on(deezer.search(&q_artist, &q_title, track.duration)) {
            Ok(Some(hit)) => Some(hit),
            Ok(None) => None,
            Err(e) => {
                warn!("metadata sync failed for {}: {e}", track.path);
                None
            }
        };

        if let Some(hit) = hit {
            let cover_key = hit
                .album_id
                .clone()
                .unwrap_or_else(|| CoverCache::cache_key(&hit.artist, &hit.album));
            let cover_file = cache_dir.join("covers").join(format!("{cover_key}.jpg"));
            let mut cover = None;
            // Album-level reuse: if we already fetched this album's cover this
            // run, point at the existing file instead of re-downloading.
            if cover_file.exists() {
                cover = std::fs::read(&cover_file).ok();
            }
            if cover.is_none()
                && let Some(ref url) = hit.cover_url
            {
                cover = rt.block_on(deezer.download_cover(url));
            }
            let cover_mime = cover.as_ref().map(|_| "image/jpeg".to_string());
            let meta = MetadataToWrite {
                title: hit.title.clone(),
                artist: hit.artist.clone(),
                album: hit.album.clone(),
                genre: hit.genre.clone(),
                year: hit.year,
                track_number: hit.track_number,
            };
            if write_tags(&track.path, &meta, cover.clone().zip(cover_mime)).is_err() {
                warn!("metadata sync: failed to write tags for {}", track.path);
                continue;
            }
            if let Some(bytes) = &cover {
                if let Some(parent) = cover_file.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if std::fs::write(&cover_file, bytes).is_ok() {
                    let _ = lib.update_cover_path(track.id, &cover_file.to_string_lossy());
                }
            }
            if let Err(e) = lib.update_metadata(
                track.id,
                &MetadataPatch {
                    title: Some(hit.title),
                    artist: Some(hit.artist),
                    album: Some(hit.album),
                    genre: hit.genre,
                    year: hit.year,
                    track_number: hit.track_number,
                    album_id: hit.album_id,
                },
            ) {
                warn!("metadata sync: failed to update DB for {}: {e}", track.path);
            }
            synced += 1;
        } else {
            let (cleaned_artist, cleaned_title) = clean_filename_stem(stem);
            if !cleaned_title.is_empty() || cleaned_artist.is_some() {
                let patch = MetadataPatch {
                    title: (!cleaned_title.is_empty()).then(|| sanitize_text(&cleaned_title)),
                    artist: cleaned_artist.map(|a| sanitize_text(&a)),
                    ..Default::default()
                };
                if patch.title.is_some() || patch.artist.is_some() {
                    if let Err(e) = lib.update_metadata(track.id, &patch) {
                        warn!(
                            "metadata sync: failed to write fallback for {}: {e}",
                            track.path
                        );
                    } else {
                        synced += 1;
                    }
                }
            }
        }
        progress.synced.store(synced, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok((synced, total))
}
