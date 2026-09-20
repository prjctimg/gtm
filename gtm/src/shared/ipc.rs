// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// IPC protocol: wire format with explicit cmd/event fields per gtm protocol v2
//
// This is free software released under the GPL-3.0 license.

use crate::shared::global::{DaemonState, EqPreset, LoudnessMode, RepeatMode, YTFilter};
use crate::shared::playlist::PlaylistFormatKind;
use crate::shared::podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};
use crate::shared::radio::{RadioCountry, RadioStation, RadioTag};
use crate::shared::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use crate::shared::subsonic::{
    SubsonicAlbum, SubsonicSearchResults, SubsonicStatus, SubsonicTrack,
};
use crate::shared::track::{LrcData, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::HashMap;

/// Extract a sub-value from a JSON object, returning `Value::Null` if missing.
/// Avoids the intermediate `.cloned().unwrap_or(Value::Null)` pattern.
#[inline]
fn field(data: &Value, key: &str) -> Value {
    data.get(key).cloned().unwrap_or(Value::Null)
}

/// Extract a string field from a JSON object, returning the default if missing.
#[inline]
fn field_str<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Default local callback port for the Spotify OAuth redirect when the caller
/// doesn't specify one.
fn default_oauth_port() -> u16 {
    8990
}

/// `/queue` sub-commands. Internally tagged via `action`, wire encoding is
/// flat: `{"action":"add","path":"...","position":null}` per `commands.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum QueueAction {
    List,
    Clear,
    Remove {
        index: u64,
    },
    Move {
        from: u64,
        to: u64,
    },
    Add {
        paths: Vec<String>,
        position: Option<u64>,
    },
    Set {
        paths: Vec<String>,
        start_idx: u64,
    },
}

/// `/library` sub-commands. Internally tagged via `action`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LibraryAction {
    Scan {
        path: String,
    },
    GetTracks {
        filter: Option<String>,
        sort: Option<String>,
    },
    GetMostPlayed {
        limit: u64,
    },
    GetRecentlyPlayed {
        limit: u64,
    },
    GetRecentlyAdded {
        limit: u64,
    },
    GetPlaylists,
    GetPlaylistTracks {
        id: i64,
    },
    CreatePlaylist {
        name: String,
    },
    RenamePlaylist {
        id: i64,
        name: String,
    },
    DeletePlaylist {
        id: i64,
    },
    AddToPlaylist {
        playlist_id: i64,
        track_ids: Vec<i64>,
    },
    ImportPlaylist {
        path: String,
        format: PlaylistFormatKind,
    },
    ExportPlaylist {
        playlist_id: i64,
        path: String,
        format: PlaylistFormatKind,
    },
    GetRecent {
        count: u64,
    },
    SyncCovers,
    SyncLyrics,
    /// Enrich unreliable track metadata via Deezer and embed tags into the
    /// files. With `path` given only that track is processed; otherwise all
    /// library tracks whose metadata is "unreliable" (title equals the raw
    /// filename stem, or artist/album missing).
    SyncMetadata {
        path: Option<String>,
    },
    /// Read the progress of the currently running library sync, if any.
    SyncStatus,
    RemoveFromPlaylist {
        playlist_id: i64,
        track_id: i64,
    },
    /// Remove duplicate entries from a playlist; returns the count removed.
    PlaylistDedup {
        playlist_id: i64,
    },
    /// Remove entries whose audio file no longer exists on disk; returns the
    /// count removed.
    PlaylistDoctor {
        playlist_id: i64,
    },
    /// Sort a playlist's tracks in place (`title` | `artist` | `album` | `date`).
    PlaylistSort {
        playlist_id: i64,
        field: String,
    },
    RemoveTrack {
        id: i64,
    },
    UpdateMetadata {
        track_id: i64,
        patch: MetadataPatch,
    },
}

/// Optional metadata fields for an edit-metadata request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetadataPatch {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    #[serde(default)]
    pub album_id: Option<String>,
}

/// Which library sync operation is (or was last) running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncKind {
    Covers,
    Lyrics,
    Metadata,
}

/// Which on-disk cache to erase via [`DaemonReq::ClearCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    Lyrics,
    Covers,
}

/// Wire request: client -> daemon.
///
/// Per `protocol.md`, the on-the-wire shape is:
/// `{"id":<u64>,"cmd":"<name>", ...params}` where `params` is flattened
/// into the top-level object (no wrapper key). We therefore serialize each
/// `DaemonReq` variant as a flat map via `#[serde(untagged)]` and flatten
/// the nested `action` enum for `Queue`/`Library`.
///
/// Deserialization of `DaemonReq` directly from params is ambiguous for
/// unit variants; the daemon must dispatch on the `cmd` string via
/// [`DaemonReq::parse_cmd`] instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DaemonReq {
    Play {
        path: String,
        start_pos: f64,
    },
    PlayStream {
        url: String,
    },
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek {
        position_secs: f64,
    },
    SetVolume {
        volume: u8,
    },
    GetVolume,
    ToggleShuffle,
    CycleRepeat {
        mode: RepeatMode,
    },
    ToggleMute,
    SetMono {
        enabled: bool,
    },
    Crossfade {
        enabled: bool,
        duration_secs: u8,
    },
    SetLoudnessMode {
        mode: LoudnessMode,
    },
    ScanLoudness {
        track_ids: Option<Vec<i64>>,
        force: Option<bool>,
    },
    SetPreGain {
        pre_gain_db: f32,
    },
    SetGapless {
        enabled: bool,
    },
    SetDynamicMode {
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    },
    SetScrobble {
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    },
    SetEqPreset {
        preset: EqPreset,
    },
    SetEqEnabled {
        enabled: bool,
    },
    SetReverb {
        enabled: bool,
        room_size: f32,
    },
    SetSpeed {
        rate: f32,
    },
    GetSpeed,
    ListEqPresets,
    Queue {
        #[serde(flatten)]
        action: QueueAction,
    },
    Library {
        #[serde(flatten)]
        action: LibraryAction,
    },
    Search {
        query: String,
    },
    GetFavourites,
    AddFavourite {
        track_id: i64,
    },
    RemoveFavourite {
        track_id: i64,
    },
    YtSearch {
        query: String,
        filter: Option<YTFilter>,
    },
    YtSearchPoll,
    YtSearchCancel,
    YtResolveStream {
        url: String,
    },
    YtDownload {
        url: String,
        title: Option<String>,
        channel: Option<String>,
    },
    YtDownloadPoll,
    YtCancelDownload {
        url: String,
    },
    YtFetchPlaylist {
        url: String,
    },
    YtFetchPlaylistPoll,
    YtSetConfig {
        cookie_source: Option<String>,
        cookie_file: Option<String>,
        js_runtime: Option<String>,
        download_dir: Option<String>,
        max_concurrent: Option<u32>,
    },
    GetCoverArt {
        track_id: i64,
        path: Option<String>,
    },
    GetArtistCoverArt {
        artist: String,
    },
    SetCoverProvider {
        provider: String,
    },
    GetLyrics {
        track_id: i64,
        path: Option<String>,
    },
    LyricsSearch {
        artist: String,
        title: String,
    },
    SpotifySetToken {
        token: String,
    },
    SpotifyOauthStart {
        client_id: String,
        port: u16,
    },
    SpotifyCancelOauth,
    SpotifyClear,
    SpotifyStatus,
    SpotifyPlayPause,
    SpotifySync,
    SpotifyPlaylists,
    SpotifyPlaylistTracks {
        id: String,
    },
    SpotifyResolve {
        playlist_id: String,
        track_index: usize,
    },
    SpotifySearchWeb {
        query: String,
    },
    /// Resolve an album found by web search to its full track list, so the
    /// TUI can queue and play it.
    SpotifyAlbumTracks {
        uri: String,
    },
    /// Resolve an artist found by web search to their top tracks.
    SpotifyArtistTopTracks {
        uri: String,
    },
    SpotifyResolveTrack {
        name: String,
        artists: String,
        album: String,
        /// `spotify:track:` URI when the caller already knows it (e.g. web
        /// search hits), enabling native librespot streaming on Premium.
        #[serde(default)]
        uri: Option<String>,
    },
    /// Play every track of a synced Spotify playlist. `shuffle` randomises the
    /// track order before enqueueing so `S` on a playlist becomes "shuffle all".
    SpotifyPlayAll {
        playlist_id: String,
        shuffle: bool,
    },
    /// Fetch the raw bytes (base64) for a Spotify album-cover URL, used to
    /// render cover art in the Spotify search picker preview.
    SpotifyTrackImage {
        image_url: String,
    },
    LastfmSetConfig {
        enabled: bool,
        api_key: Option<String>,
        api_secret: Option<String>,
        session_key: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    },
    LastfmAuthUrl,
    LastfmAuthenticate {
        token: String,
    },
    LastfmStatus,
    LastfmClear,
    /// Love the current track on Last.fm; also immediate-scrobbles the active
    /// play session so a loved track is never lost on a quick skip.
    LastfmLove,
    /// Un-love the current track on Last.fm.
    LastfmUnlove,
    SetSleepTimer {
        minutes: u32,
    },
    CancelSleepTimer,
    /// Enter / leave low-power mode (pauses playback, eases up on background work).
    SetLowPower {
        enabled: bool,
    },
    /// Read the current low-power mode.
    GetLowPower,
    /// List available output device names.
    ListAudioDevices,
    /// Switch the active output device (`None` = system default).
    SetAudioDevice {
        name: Option<String>,
    },
    ClearCache {
        what: CacheKind,
    },
    /// Configure a Navidrome/Subsonic server (validated by a ping).
    SubsonicConfigure {
        server: String,
        username: String,
        password: String,
    },
    /// Store (or clear, when empty) the Deezer streaming ARL token. Persisted
    /// via the OS keychain so full-track streaming survives daemon restarts.
    SetDeezerArl {
        arl: String,
    },
    /// Forget the Subsonic server configuration.
    SubsonicClear,
    SubsonicStatus,
    SubsonicPing,
    SubsonicSearch {
        query: String,
    },
    SubsonicAlbums {
        offset: u64,
        size: u64,
    },
    SubsonicAlbumTracks {
        album_id: String,
    },
    /// Play a Subsonic track natively by streaming its `/rest/stream` URL.
    SubsonicPlay {
        track_id: String,
        title: String,
        artist: String,
        album: String,
        duration_secs: Option<u64>,
        cover_url: Option<String>,
    },
    /// Enqueue every track of an album (metadata attached) and play the first.
    SubsonicPlayAlbum {
        album_id: String,
    },
    /// Base64 cover art bytes for a Subsonic track (via getCoverArt).
    SubsonicCover {
        track_id: String,
    },
    /// Subscribe to a podcast RSS/Atom feed.
    PodcastAddFeed {
        url: String,
    },
    PodcastRemoveFeed {
        feed_id: String,
    },
    PodcastFeeds,
    PodcastEpisodes {
        feed_id: String,
    },
    /// Re-fetch one feed (`feed_id` set) or every subscription (`None`).
    PodcastRefresh {
        feed_id: Option<String>,
    },
    PodcastStatus,
    PodcastPlay {
        feed_id: String,
        episode_index: usize,
    },
    RadioSearch {
        query: String,
        limit: u16,
    },
    RadioTop {
        limit: u16,
    },
    RadioPlay {
        station_id: String,
        station_name: String,
    },
    RadioTags {
        limit: u16,
    },
    RadioByTag {
        tag: String,
        limit: u16,
    },
    RadioCountries {
        limit: u16,
    },
    RadioByCountry {
        country: String,
        limit: u16,
    },
    /// List available chart sources (Spotify, future Deezer/Tidal).
    ChartsSources,
    /// List charts from a specific source or all configured sources.
    ChartsList {
        source_id: Option<String>,
    },
    /// Fetch tracks for a specific chart playlist.
    ChartsTracks {
        source_id: String,
        chart_id: String,
    },
    GetStatus,
    /// Like `GetStatus` but omits the full `default_list` (the whole library)
    /// from the returned state. Used for the client's periodic background
    /// refresh so the daemon does not re-serialize the library every second.
    GetStatusLite,
    CheckHealth,
    Ping,
    Quit,
}

impl DaemonReq {
    /// Canonical `cmd` string for this request, as defined by `commands.md`.
    pub fn cmd_name(&self) -> &'static str {
        match self {
            DaemonReq::Play { .. } => "play",
            DaemonReq::PlayStream { .. } => "play_stream",
            DaemonReq::PlayPause => "play_pause",
            DaemonReq::Pause => "pause",
            DaemonReq::Stop => "stop",
            DaemonReq::Next => "next",
            DaemonReq::Prev => "prev",
            DaemonReq::Seek { .. } => "seek",
            DaemonReq::SetVolume { .. } => "set_volume",
            DaemonReq::GetVolume => "get_volume",
            DaemonReq::ToggleShuffle => "toggle_shuffle",
            DaemonReq::CycleRepeat { .. } => "cycle_repeat",
            DaemonReq::ToggleMute => "toggle_mute",
            DaemonReq::SetMono { .. } => "set_mono",
            DaemonReq::Crossfade { .. } => "crossfade",
            DaemonReq::SetLoudnessMode { .. } => "set_loudness_mode",
            DaemonReq::ScanLoudness { .. } => "scan_loudness",
            DaemonReq::SetPreGain { .. } => "set_pre_gain",
            DaemonReq::SetGapless { .. } => "set_gapless",
            DaemonReq::SetDynamicMode { .. } => "set_dynamic_mode",
            DaemonReq::SetScrobble { .. } => "set_scrobble",
            DaemonReq::SetEqPreset { .. } => "set_eq_preset",
            DaemonReq::SetEqEnabled { .. } => "set_eq_enabled",
            DaemonReq::SetReverb { .. } => "set_reverb",
            DaemonReq::SetSpeed { .. } => "set_speed",
            DaemonReq::GetSpeed => "get_speed",
            DaemonReq::ListEqPresets => "list_eq_presets",
            DaemonReq::Queue { .. } => "queue",
            DaemonReq::Library { .. } => "library",
            DaemonReq::Search { .. } => "search",
            DaemonReq::GetFavourites => "get_favourites",
            DaemonReq::AddFavourite { .. } => "add_favourite",
            DaemonReq::RemoveFavourite { .. } => "remove_favourite",
            DaemonReq::YtSearch { .. } => "yt_search",
            DaemonReq::YtSearchPoll => "yt_search_poll",
            DaemonReq::YtSearchCancel => "yt_search_cancel",
            DaemonReq::YtResolveStream { .. } => "yt_resolve_stream",
            DaemonReq::YtDownload { .. } => "yt_download",
            DaemonReq::YtDownloadPoll => "yt_download_poll",
            DaemonReq::YtCancelDownload { .. } => "yt_cancel_download",
            DaemonReq::YtFetchPlaylist { .. } => "yt_fetch_playlist",
            DaemonReq::YtFetchPlaylistPoll => "yt_playlist_poll",
            DaemonReq::YtSetConfig { .. } => "yt_set_config",
            DaemonReq::GetCoverArt { .. } => "get_cover_art",
            DaemonReq::GetArtistCoverArt { .. } => "artist_cover_art",
            DaemonReq::SetCoverProvider { .. } => "set_cover_provider",
            DaemonReq::GetLyrics { .. } => "get_lyrics",
            DaemonReq::LyricsSearch { .. } => "lyrics_search",
            DaemonReq::SpotifySetToken { .. } => "spotify_set_token",
            DaemonReq::SpotifyOauthStart { .. } => "spotify_oauth_start",
            DaemonReq::SpotifyCancelOauth => "spotify_cancel_oauth",
            DaemonReq::SpotifyClear => "spotify_clear",
            DaemonReq::SpotifyStatus => "spotify_status",
            DaemonReq::SpotifyPlayPause => "spotify_play_pause",
            DaemonReq::SpotifySync => "spotify_sync",
            DaemonReq::SpotifyPlaylists => "spotify_playlists",
            DaemonReq::SpotifyPlaylistTracks { .. } => "spotify_playlist_tracks",
            DaemonReq::SpotifyResolve { .. } => "spotify_resolve",
            DaemonReq::SpotifySearchWeb { .. } => "spotify_search_web",
            DaemonReq::SpotifyAlbumTracks { .. } => "spotify_album_tracks",
            DaemonReq::SpotifyArtistTopTracks { .. } => "spotify_artist_top_tracks",
            DaemonReq::SpotifyResolveTrack { .. } => "spotify_resolve_track",
            DaemonReq::SpotifyPlayAll { .. } => "spotify_play_all",
            DaemonReq::SpotifyTrackImage { .. } => "spotify_track_image",
            DaemonReq::LastfmSetConfig { .. } => "lastfm_set_config",
            DaemonReq::LastfmAuthUrl => "lastfm_auth_url",
            DaemonReq::LastfmAuthenticate { .. } => "lastfm_authenticate",
            DaemonReq::LastfmStatus => "lastfm_status",
            DaemonReq::LastfmClear => "lastfm_clear",
            DaemonReq::LastfmLove => "lastfm_love",
            DaemonReq::LastfmUnlove => "lastfm_unlove",
            DaemonReq::SetSleepTimer { .. } => "set_sleep_timer",
            DaemonReq::CancelSleepTimer => "cancel_sleep_timer",
            DaemonReq::SetLowPower { .. } => "set_low_power",
            DaemonReq::GetLowPower => "get_low_power",
            DaemonReq::ListAudioDevices => "list_audio_devices",
            DaemonReq::SetAudioDevice { .. } => "set_audio_device",
            DaemonReq::ClearCache { .. } => "clear_cache",
            DaemonReq::SubsonicConfigure { .. } => "subsonic_configure",
            DaemonReq::SetDeezerArl { .. } => "set_deezer_arl",
            DaemonReq::SubsonicClear => "subsonic_clear",
            DaemonReq::SubsonicStatus => "subsonic_status",
            DaemonReq::SubsonicPing => "subsonic_ping",
            DaemonReq::SubsonicSearch { .. } => "subsonic_search",
            DaemonReq::SubsonicAlbums { .. } => "subsonic_albums",
            DaemonReq::SubsonicAlbumTracks { .. } => "subsonic_album_tracks",
            DaemonReq::SubsonicPlay { .. } => "subsonic_play",
            DaemonReq::SubsonicPlayAlbum { .. } => "subsonic_play_album",
            DaemonReq::SubsonicCover { .. } => "subsonic_cover",
            DaemonReq::PodcastAddFeed { .. } => "podcast_add_feed",
            DaemonReq::PodcastRemoveFeed { .. } => "podcast_remove_feed",
            DaemonReq::PodcastFeeds => "podcast_feeds",
            DaemonReq::PodcastEpisodes { .. } => "podcast_episodes",
            DaemonReq::PodcastRefresh { .. } => "podcast_refresh",
            DaemonReq::PodcastStatus => "podcast_status",
            DaemonReq::PodcastPlay { .. } => "podcast_play",
            DaemonReq::RadioSearch { .. } => "radio_search",
            DaemonReq::RadioTop { .. } => "radio_top",
            DaemonReq::RadioPlay { .. } => "radio_play",
            DaemonReq::RadioTags { .. } => "radio_tags",
            DaemonReq::RadioByTag { .. } => "radio_bytag",
            DaemonReq::RadioCountries { .. } => "radio_countries",
            DaemonReq::RadioByCountry { .. } => "radio_bycountry",
            DaemonReq::ChartsSources => "charts_sources",
            DaemonReq::ChartsList { .. } => "charts_list",
            DaemonReq::ChartsTracks { .. } => "charts_tracks",
            DaemonReq::GetStatus => "get_status",
            DaemonReq::GetStatusLite => "get_status_lite",
            DaemonReq::CheckHealth => "check_health",
            DaemonReq::Ping => "ping",
            DaemonReq::Quit => "quit",
        }
    }

    /// Reconstruct a `DaemonReq` from a parsed `cmd` string and the flat
    /// `params` object delivered on the wire. The dispatch is explicit so
    /// unit variants (`Pause`, `Stop`, ...) disambiguate cleanly: something
    /// `#[serde(untagged)]` cannot do by itself.
    pub fn parse_cmd(cmd: &str, params: Value) -> std::result::Result<Self, String> {
        fn p<T: DeserializeOwned>(v: Value) -> std::result::Result<T, String> {
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        Ok(match cmd {
            "play" => {
                #[derive(Deserialize)]
                struct Params {
                    path: String,
                    start_pos: f64,
                }
                let x: Params = p(params)?;
                DaemonReq::Play {
                    path: x.path,
                    start_pos: x.start_pos,
                }
            }
            "play_stream" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::PlayStream { url: x.url }
            }
            "play_pause" => DaemonReq::PlayPause,
            "pause" => DaemonReq::Pause,
            "stop" => DaemonReq::Stop,
            "next" => DaemonReq::Next,
            "prev" => DaemonReq::Prev,
            "seek" => {
                #[derive(Deserialize)]
                struct Params {
                    position_secs: f64,
                }
                let x: Params = p(params)?;
                DaemonReq::Seek {
                    position_secs: x.position_secs,
                }
            }
            "set_volume" => {
                #[derive(Deserialize)]
                struct Params {
                    volume: u8,
                }
                let x: Params = p(params)?;
                DaemonReq::SetVolume { volume: x.volume }
            }
            "get_volume" => DaemonReq::GetVolume,
            "toggle_shuffle" => DaemonReq::ToggleShuffle,
            "cycle_repeat" => {
                #[derive(Deserialize)]
                struct Params {
                    mode: RepeatMode,
                }
                let x: Params = p(params)?;
                DaemonReq::CycleRepeat { mode: x.mode }
            }
            "toggle_mute" => DaemonReq::ToggleMute,
            "set_mono" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetMono { enabled: x.enabled }
            }
            "crossfade" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    duration_secs: u8,
                }
                let x: Params = p(params)?;
                DaemonReq::Crossfade {
                    enabled: x.enabled,
                    duration_secs: x.duration_secs,
                }
            }
            "set_eq_preset" => {
                #[derive(Deserialize)]
                struct Params {
                    preset: EqPreset,
                }
                let x: Params = p(params)?;
                DaemonReq::SetEqPreset { preset: x.preset }
            }
            "set_eq_enabled" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetEqEnabled { enabled: x.enabled }
            }
            "set_reverb" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    room_size: f32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetReverb {
                    enabled: x.enabled,
                    room_size: x.room_size,
                }
            }
            "list_eq_presets" => DaemonReq::ListEqPresets,
            "set_speed" => {
                #[derive(Deserialize)]
                struct Params {
                    rate: f32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetSpeed { rate: x.rate }
            }
            "get_speed" => DaemonReq::GetSpeed,
            "queue" => DaemonReq::Queue { action: p(params)? },
            "library" => DaemonReq::Library { action: p(params)? },
            "search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                }
                let x: Params = p(params)?;
                DaemonReq::Search { query: x.query }
            }
            "get_favourites" => DaemonReq::GetFavourites,
            "add_favourite" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::AddFavourite {
                    track_id: x.track_id,
                }
            }
            "remove_favourite" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::RemoveFavourite {
                    track_id: x.track_id,
                }
            }
            "yt_search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                    filter: Option<YTFilter>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtSearch {
                    query: x.query,
                    filter: x.filter,
                }
            }
            "yt_search_poll" => DaemonReq::YtSearchPoll,
            "yt_search_cancel" => DaemonReq::YtSearchCancel,
            "yt_resolve_stream" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtResolveStream { url: x.url }
            }
            "yt_download" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                    title: Option<String>,
                    channel: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtDownload {
                    url: x.url,
                    title: x.title,
                    channel: x.channel,
                }
            }
            "yt_download_poll" => DaemonReq::YtDownloadPoll,
            "yt_cancel_download" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtCancelDownload { url: x.url }
            }
            "yt_fetch_playlist" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtFetchPlaylist { url: x.url }
            }
            "yt_playlist_poll" => DaemonReq::YtFetchPlaylistPoll,
            "yt_set_config" => {
                #[derive(Deserialize)]
                struct Params {
                    cookie_source: Option<String>,
                    cookie_file: Option<String>,
                    js_runtime: Option<String>,
                    download_dir: Option<String>,
                    max_concurrent: Option<u32>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtSetConfig {
                    cookie_source: x.cookie_source,
                    cookie_file: x.cookie_file,
                    js_runtime: x.js_runtime,
                    download_dir: x.download_dir,
                    max_concurrent: x.max_concurrent,
                }
            }
            "get_cover_art" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                    #[serde(default)]
                    path: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::GetCoverArt {
                    track_id: x.track_id,
                    path: x.path,
                }
            }
            "artist_cover_art" => {
                #[derive(Deserialize)]
                struct Params {
                    artist: String,
                }
                let x: Params = p(params)?;
                DaemonReq::GetArtistCoverArt { artist: x.artist }
            }
            "set_cover_provider" => {
                #[derive(Deserialize)]
                struct Params {
                    provider: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SetCoverProvider {
                    provider: x.provider,
                }
            }
            "get_lyrics" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                    path: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::GetLyrics {
                    track_id: x.track_id,
                    path: x.path,
                }
            }
            "lyrics_search" => {
                #[derive(Deserialize)]
                struct Params {
                    artist: String,
                    title: String,
                }
                let x: Params = p(params)?;
                DaemonReq::LyricsSearch {
                    artist: x.artist,
                    title: x.title,
                }
            }
            "spotify_set_token" => {
                #[derive(Deserialize)]
                struct Params {
                    token: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifySetToken { token: x.token }
            }
            "spotify_oauth_start" => {
                #[derive(Deserialize)]
                struct Params {
                    client_id: String,
                    #[serde(default = "default_oauth_port")]
                    port: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyOauthStart {
                    client_id: x.client_id,
                    port: x.port,
                }
            }
            "spotify_cancel_oauth" => DaemonReq::SpotifyCancelOauth,
            "spotify_clear" => DaemonReq::SpotifyClear,
            "spotify_status" => DaemonReq::SpotifyStatus,
            "spotify_play_pause" => DaemonReq::SpotifyPlayPause,
            "spotify_sync" => DaemonReq::SpotifySync,
            "spotify_playlists" => DaemonReq::SpotifyPlaylists,
            "spotify_playlist_tracks" => {
                #[derive(Deserialize)]
                struct Params {
                    id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyPlaylistTracks { id: x.id }
            }
            "spotify_resolve" => {
                #[derive(Deserialize)]
                struct Params {
                    playlist_id: String,
                    track_index: usize,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyResolve {
                    playlist_id: x.playlist_id,
                    track_index: x.track_index,
                }
            }
            "spotify_search_web" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifySearchWeb { query: x.query }
            }
            "spotify_album_tracks" => {
                #[derive(Deserialize)]
                struct Params {
                    uri: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyAlbumTracks { uri: x.uri }
            }
            "spotify_artist_top_tracks" => {
                #[derive(Deserialize)]
                struct Params {
                    uri: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyArtistTopTracks { uri: x.uri }
            }
            "spotify_resolve_track" => {
                #[derive(Deserialize)]
                struct Params {
                    name: String,
                    artists: String,
                    album: String,
                    #[serde(default)]
                    uri: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyResolveTrack {
                    name: x.name,
                    artists: x.artists,
                    album: x.album,
                    uri: x.uri,
                }
            }
            "spotify_play_all" => {
                #[derive(Deserialize)]
                struct Params {
                    playlist_id: String,
                    #[serde(default)]
                    shuffle: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyPlayAll {
                    playlist_id: x.playlist_id,
                    shuffle: x.shuffle,
                }
            }
            "spotify_track_image" => {
                #[derive(Deserialize)]
                struct Params {
                    image_url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SpotifyTrackImage {
                    image_url: x.image_url,
                }
            }
            "lastfm_set_config" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    api_key: Option<String>,
                    api_secret: Option<String>,
                    session_key: Option<String>,
                    min_play_secs: Option<u32>,
                    min_play_pct: Option<f32>,
                }
                let x: Params = p(params)?;
                DaemonReq::LastfmSetConfig {
                    enabled: x.enabled,
                    api_key: x.api_key,
                    api_secret: x.api_secret,
                    session_key: x.session_key,
                    min_play_secs: x.min_play_secs,
                    min_play_pct: x.min_play_pct,
                }
            }
            "lastfm_auth_url" => DaemonReq::LastfmAuthUrl,
            "lastfm_authenticate" => {
                #[derive(Deserialize)]
                struct Params {
                    token: String,
                }
                let x: Params = p(params)?;
                DaemonReq::LastfmAuthenticate { token: x.token }
            }
            "lastfm_status" => DaemonReq::LastfmStatus,
            "lastfm_clear" => DaemonReq::LastfmClear,
            "lastfm_love" => DaemonReq::LastfmLove,
            "lastfm_unlove" => DaemonReq::LastfmUnlove,
            "set_sleep_timer" => {
                #[derive(Deserialize)]
                struct Params {
                    minutes: u32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetSleepTimer { minutes: x.minutes }
            }
            "cancel_sleep_timer" => DaemonReq::CancelSleepTimer,
            "set_low_power" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetLowPower { enabled: x.enabled }
            }
            "get_low_power" => DaemonReq::GetLowPower,
            "list_audio_devices" => DaemonReq::ListAudioDevices,
            "set_audio_device" => {
                #[derive(Deserialize)]
                struct Params {
                    name: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::SetAudioDevice { name: x.name }
            }
            "clear_cache" => {
                #[derive(Deserialize)]
                struct Params {
                    what: CacheKind,
                }
                let x: Params = p(params)?;
                DaemonReq::ClearCache { what: x.what }
            }
            "subsonic_configure" => {
                #[derive(Deserialize)]
                struct Params {
                    server: String,
                    username: String,
                    password: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicConfigure {
                    server: x.server,
                    username: x.username,
                    password: x.password,
                }
            }
            "set_deezer_arl" => {
                #[derive(Deserialize)]
                struct Params {
                    arl: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SetDeezerArl { arl: x.arl }
            }
            "subsonic_clear" => DaemonReq::SubsonicClear,
            "subsonic_status" => DaemonReq::SubsonicStatus,
            "subsonic_ping" => DaemonReq::SubsonicPing,
            "subsonic_search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicSearch { query: x.query }
            }
            "subsonic_albums" => {
                #[derive(Deserialize)]
                struct Params {
                    offset: u64,
                    size: u64,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicAlbums {
                    offset: x.offset,
                    size: x.size,
                }
            }
            "subsonic_album_tracks" => {
                #[derive(Deserialize)]
                struct Params {
                    album_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicAlbumTracks {
                    album_id: x.album_id,
                }
            }
            "subsonic_play" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: String,
                    title: String,
                    artist: String,
                    album: String,
                    duration_secs: Option<u64>,
                    cover_url: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicPlay {
                    track_id: x.track_id,
                    title: x.title,
                    artist: x.artist,
                    album: x.album,
                    duration_secs: x.duration_secs,
                    cover_url: x.cover_url,
                }
            }
            "subsonic_cover" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicCover {
                    track_id: x.track_id,
                }
            }
            "subsonic_play_album" => {
                #[derive(Deserialize)]
                struct Params {
                    album_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::SubsonicPlayAlbum {
                    album_id: x.album_id,
                }
            }
            "podcast_add_feed" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::PodcastAddFeed { url: x.url }
            }
            "podcast_remove_feed" => {
                #[derive(Deserialize)]
                struct Params {
                    feed_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::PodcastRemoveFeed { feed_id: x.feed_id }
            }
            "podcast_feeds" => DaemonReq::PodcastFeeds,
            "podcast_episodes" => {
                #[derive(Deserialize)]
                struct Params {
                    feed_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::PodcastEpisodes { feed_id: x.feed_id }
            }
            "podcast_refresh" => {
                #[derive(Deserialize)]
                struct Params {
                    feed_id: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::PodcastRefresh { feed_id: x.feed_id }
            }
            "podcast_status" => DaemonReq::PodcastStatus,
            "podcast_play" => {
                #[derive(Deserialize)]
                struct Params {
                    feed_id: String,
                    episode_index: usize,
                }
                let x: Params = p(params)?;
                DaemonReq::PodcastPlay {
                    feed_id: x.feed_id,
                    episode_index: x.episode_index,
                }
            }
            "radio_search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioSearch {
                    query: x.query,
                    limit: x.limit,
                }
            }
            "radio_top" => {
                #[derive(Deserialize)]
                struct Params {
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioTop { limit: x.limit }
            }
            "radio_play" => {
                #[derive(Deserialize)]
                struct Params {
                    station_id: String,
                    station_name: String,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioPlay {
                    station_id: x.station_id,
                    station_name: x.station_name,
                }
            }
            "radio_tags" => {
                #[derive(Deserialize)]
                struct Params {
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioTags { limit: x.limit }
            }
            "radio_bytag" => {
                #[derive(Deserialize)]
                struct Params {
                    tag: String,
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioByTag {
                    tag: x.tag,
                    limit: x.limit,
                }
            }
            "radio_countries" => {
                #[derive(Deserialize)]
                struct Params {
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioCountries { limit: x.limit }
            }
            "radio_bycountry" => {
                #[derive(Deserialize)]
                struct Params {
                    country: String,
                    limit: u16,
                }
                let x: Params = p(params)?;
                DaemonReq::RadioByCountry {
                    country: x.country,
                    limit: x.limit,
                }
            }
            "charts_sources" => DaemonReq::ChartsSources,
            "charts_list" => {
                #[derive(Deserialize)]
                struct Params {
                    source_id: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::ChartsList {
                    source_id: x.source_id,
                }
            }
            "charts_tracks" => {
                #[derive(Deserialize)]
                struct Params {
                    source_id: String,
                    chart_id: String,
                }
                let x: Params = p(params)?;
                DaemonReq::ChartsTracks {
                    source_id: x.source_id,
                    chart_id: x.chart_id,
                }
            }
            "get_status" => DaemonReq::GetStatus,
            "get_status_lite" => DaemonReq::GetStatusLite,
            "check_health" => DaemonReq::CheckHealth,
            "ping" => DaemonReq::Ping,
            "set_loudness_mode" => {
                #[derive(Deserialize)]
                struct Params {
                    mode: LoudnessMode,
                }
                let x: Params = p(params)?;
                DaemonReq::SetLoudnessMode { mode: x.mode }
            }
            "scan_loudness" => {
                #[derive(Deserialize)]
                struct Params {
                    track_ids: Option<Vec<i64>>,
                    force: Option<bool>,
                }
                let x: Params = p(params)?;
                DaemonReq::ScanLoudness {
                    track_ids: x.track_ids,
                    force: x.force,
                }
            }
            "set_pre_gain" => {
                #[derive(Deserialize)]
                struct Params {
                    pre_gain_db: f32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetPreGain {
                    pre_gain_db: x.pre_gain_db,
                }
            }
            "set_gapless" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetGapless { enabled: x.enabled }
            }
            "set_dynamic_mode" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    min_queue_remaining: Option<u32>,
                    max_history: Option<u32>,
                }
                let x: Params = p(params)?;
                DaemonReq::SetDynamicMode {
                    enabled: x.enabled,
                    min_queue_remaining: x.min_queue_remaining,
                    max_history: x.max_history,
                }
            }
            "set_scrobble" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    api_key: Option<String>,
                    session_token: Option<String>,
                    min_play_secs: Option<u32>,
                    min_play_pct: Option<f32>,
                }
                let x: Params = p(params)?;
                DaemonReq::SetScrobble {
                    enabled: x.enabled,
                    api_key: x.api_key,
                    session_token: x.session_token,
                    min_play_secs: x.min_play_secs,
                    min_play_pct: x.min_play_pct,
                }
            }
            "quit" => DaemonReq::Quit,
            other => return Err(format!("unknown command: {other}")),
        })
    }
}

/// Wire request: client -> daemon.
///
/// `params` is a nested object so command fields can never collide with the
/// envelope's own `id`/`cmd` keys (e.g. `library`/`remove_track` carries its
/// own `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReq {
    pub id: u64,
    pub cmd: String,
    pub params: Value,
}

/// Wire response: daemon -> client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRes {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub data: Option<Value>,
}

impl WireRes {
    pub fn ok(id: u64, data: Option<Value>) -> Self {
        Self {
            id,
            ok: Some(true),
            error: None,
            data,
        }
    }

    pub fn err(id: u64, error: String) -> Self {
        Self {
            id,
            ok: Some(false),
            error: Some(error),
            data: None,
        }
    }
}

/// Wire event: daemon -> client (broadcast).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEvent {
    pub event: String,
    #[serde(flatten)]
    pub data: Value,
}

impl WireEvent {
    pub fn new(event: &str, data: Value) -> Self {
        Self {
            event: event.to_string(),
            data,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum DaemonEvent {
    #[serde(rename = "playback_started")]
    PlaybackStarted {
        track: TrackInfo,
        auto_advanced: bool,
        time_pos: f64,
        duration: f64,
    },
    #[serde(rename = "playback_paused")]
    PlaybackPaused { time_pos: f64 },
    #[serde(rename = "playback_stopped")]
    PlaybackStopped,
    #[serde(rename = "track_ended")]
    TrackEnded,
    #[serde(rename = "position_changed")]
    PositionChanged { time_pos: f64 },
    #[serde(rename = "duration_changed")]
    DurationChanged { duration: f64 },
    #[serde(rename = "volume_changed")]
    VolumeChanged { volume: u8 },
    #[serde(rename = "mono_changed")]
    MonoChanged { enabled: bool },
    #[serde(rename = "metadata_changed")]
    MetadataChanged { detail: String },
    #[serde(rename = "queue_changed")]
    QueueChanged { queue: Vec<TrackInfo>, cursor: u64 },
    #[serde(rename = "queue_index_changed")]
    QueueIndexChanged { index: u64 },
    #[serde(rename = "repeat_mode_changed")]
    RepeatModeChanged { mode: RepeatMode },
    #[serde(rename = "shuffle_changed")]
    ShuffleChanged { enabled: bool },
    #[serde(rename = "crossfade_changed")]
    CrossfadeChanged { enabled: bool, duration_secs: u8 },
    /// Emitted once when the next track is about to enter crossfade (5s before
    /// it begins). The client animates the countdown until the crossfade
    /// starts.
    #[serde(rename = "crossfade_countdown")]
    CrossfadeCountdown { track: TrackInfo },
    #[serde(rename = "loudness_mode_changed")]
    LoudnessModeChanged { mode: LoudnessMode },
    #[serde(rename = "loudness_scan_progress")]
    LoudnessScanProgress {
        tracks_remaining: u32,
        tracks_total: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_track: Option<TrackInfo>,
    },
    #[serde(rename = "loudness_scan_done")]
    LoudnessScanDone { scanned: u32, failed: u32 },
    #[serde(rename = "pre_gain_changed")]
    PreGainChanged { pre_gain_db: f32 },
    #[serde(rename = "gapless_changed")]
    GaplessChanged { enabled: bool },
    #[serde(rename = "dynamic_mode_changed")]
    DynamicModeChanged {
        enabled: bool,
        min_queue_remaining: u32,
        max_history: u32,
    },
    #[serde(rename = "scrobble_config_changed")]
    ScrobbleConfigChanged { enabled: bool },
    #[serde(rename = "sleep_timer_tick")]
    SleepTimerTick { remaining_secs: u32 },
    #[serde(rename = "sleep_timer_expired")]
    SleepTimerExpired,
    #[serde(rename = "low_power_changed")]
    LowPowerChanged { enabled: bool },
    #[serde(rename = "audio_device_changed")]
    AudioDeviceChanged { name: Option<String> },
    #[serde(rename = "eq_preset_changed")]
    EqPresetChanged { preset: EqPreset },
    #[serde(rename = "eq_enabled_changed")]
    EqEnabledChanged { enabled: bool },
    #[serde(rename = "reverb_changed")]
    ReverbChanged { enabled: bool, room_size: f32 },
    #[serde(rename = "speed_changed")]
    SpeedChanged { rate: f32 },
    #[serde(rename = "custom")]
    Custom {
        name: String,
        data: HashMap<String, String>,
    },
    /// Spotify link state changed (e.g. an OAuth link flow completed).
    #[serde(rename = "spotify_status_changed")]
    SpotifyStatusChanged,
    #[serde(rename = "spectrum_changed")]
    SpectrumChanged { levels: Vec<f32> },
    /// Time-domain waveform ring (interleaved L/R) plus a stereo flag, for
    /// the Wave/Stereo visualizer modes. Decimated on the decode/stream
    /// threads so a ~5.5 kHz ring reaches the client at ~30 Hz.
    #[serde(rename = "waveform_changed")]
    WaveformChanged { samples: Vec<f32>, stereo: bool },
    /// A live ICY/Shoutcast stream published a new `StreamTitle` (or cleared
    /// it, `None`).
    #[serde(rename = "radio_title_changed")]
    RadioTitleChanged { title: Option<String> },
    /// Generic internet connectivity changed, as measured by the daemon's
    /// bounded TCP probes (1.1.1.1:443, then gstatic.com:443). Independent of
    /// any provider link state; drives the footer `Network` module.
    #[serde(rename = "network_status_changed")]
    NetworkStatusChanged { online: bool },
    #[serde(rename = "heartbeat")]
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRes {
    Ok,
    Value {
        value: Value,
    },
    Tracks {
        tracks: Box<Vec<TrackInfo>>,
    },
    QueueState {
        queue: Box<Vec<TrackInfo>>,
        cursor: u64,
    },
    Status {
        state: Box<DaemonState>,
    },
    Playlists {
        playlists: Vec<Playlist>,
    },
    YtSearchResults {
        query: String,
        results: Vec<YTSearchResult>,
    },
    StreamInfo {
        info: Box<StreamInfo>,
    },
    Lyrics {
        lyrics: Option<LrcData>,
    },
    SpotifyStatusRes {
        status: SpotifyStatus,
    },
    /// OAuth link flow started; the user must open this URL in a browser.
    SpotifyOauthStarted {
        url: String,
    },
    SpotifyPlaylistsRes {
        playlists: Vec<SpotifyPlaylist>,
    },
    SpotifyTracksRes {
        tracks: Vec<SpotifyTrack>,
    },
    /// Base64-encoded cover image bytes fetched from a Spotify CDN URL.
    SpotifyImageRes {
        data: Option<String>,
    },
    SubsonicStatusRes {
        status: SubsonicStatus,
    },
    SubsonicSearchRes {
        results: SubsonicSearchResults,
    },
    SubsonicAlbumsRes {
        albums: Vec<SubsonicAlbum>,
    },
    SubsonicTracksRes {
        tracks: Vec<SubsonicTrack>,
    },
    /// Ping succeeded against the configured server.
    SubsonicPingRes {
        message: String,
    },
    PodcastFeedsRes {
        feeds: Vec<PodcastFeed>,
    },
    PodcastEpisodesRes {
        feed_id: String,
        feed_title: String,
        episodes: Vec<PodcastEpisode>,
    },
    PodcastStatusRes {
        status: PodcastStatus,
    },
    RadioStationsRes {
        stations: Vec<RadioStation>,
    },
    RadioTagsRes {
        tags: Vec<RadioTag>,
    },
    RadioCountriesRes {
        countries: Vec<RadioCountry>,
    },
    ChartsSourcesRes {
        sources: Vec<crate::shared::chart::ChartSource>,
    },
    ChartsListRes {
        charts: Vec<crate::shared::chart::ChartPlaylist>,
    },
    ChartsTracksRes {
        tracks: Vec<crate::shared::chart::ChartTrack>,
    },
    ChartsLoaded {
        charts: Vec<crate::shared::chart::ChartPlaylist>,
    },
    ChartTracksLoaded {
        tracks: Vec<crate::shared::chart::ChartTrack>,
    },
    CoverArt {
        data: Option<String>,
    },
    SyncStatus {
        running: bool,
        kind: SyncKind,
        synced: usize,
        total: usize,
    },
    Pong,
    HealthReport {
        report: Box<HealthReport>,
    },
    EqPresets {
        presets: Vec<String>,
    },
    LastfmAuthUrlRes {
        url: String,
    },
    LastfmStatusRes {
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        ready: bool,
        /// Whether the currently playing track is loved on Last.fm.
        loved: bool,
    },
    YtDownloadProgress {
        id: u64,
        url: String,
        title: String,
        progress: f64,
        status: String,
        error: Option<String>,
        file_path: Option<String>,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
        rate_bps: Option<f64>,
        eta_secs: Option<u64>,
    },
    YtDownloadResult {
        id: u64,
        url: String,
        title: String,
        file_path: String,
    },
    Error {
        message: String,
    },
}

impl DaemonRes {
    /// Serialize this typed response into a `WireRes` for the command socket.
    pub fn to_wire(self, id: u64) -> WireRes {
        let data = match self {
            DaemonRes::Ok => None,
            DaemonRes::Value { value } => Some(value),
            DaemonRes::Tracks { tracks } => Some(serde_json::json!({ "tracks": tracks })),
            DaemonRes::QueueState { queue, cursor } => {
                Some(serde_json::json!({ "queue": queue, "cursor": cursor }))
            }
            DaemonRes::Status { state } => Some(serde_json::json!({ "state": state })),
            DaemonRes::Playlists { playlists } => {
                Some(serde_json::json!({ "playlists": playlists }))
            }
            DaemonRes::YtSearchResults { query, results } => {
                Some(serde_json::json!({ "query": query, "results": results }))
            }
            DaemonRes::StreamInfo { info } => Some(serde_json::json!({ "info": info })),
            DaemonRes::Lyrics { lyrics } => Some(serde_json::json!({ "lyrics": lyrics })),
            DaemonRes::SpotifyStatusRes { status } => Some(serde_json::json!({ "status": status })),
            DaemonRes::SpotifyPlaylistsRes { playlists } => {
                Some(serde_json::json!({ "playlists": playlists }))
            }
            DaemonRes::SpotifyOauthStarted { url } => Some(serde_json::json!({ "url": url })),
            DaemonRes::SpotifyTracksRes { tracks } => Some(serde_json::json!({ "tracks": tracks })),
            DaemonRes::SpotifyImageRes { data } => Some(serde_json::json!({ "data": data })),
            DaemonRes::SubsonicStatusRes { status } => {
                Some(serde_json::json!({ "status": status }))
            }
            DaemonRes::SubsonicSearchRes { results } => {
                Some(serde_json::json!({ "results": results }))
            }
            DaemonRes::SubsonicAlbumsRes { albums } => {
                Some(serde_json::json!({ "albums": albums }))
            }
            DaemonRes::SubsonicTracksRes { tracks } => {
                Some(serde_json::json!({ "tracks": tracks }))
            }
            DaemonRes::SubsonicPingRes { message } => {
                Some(serde_json::json!({ "message": message }))
            }
            DaemonRes::PodcastFeedsRes { feeds } => Some(serde_json::json!({ "feeds": feeds })),
            DaemonRes::PodcastEpisodesRes {
                feed_id,
                feed_title,
                episodes,
            } => Some(serde_json::json!({
                "feed_id": feed_id,
                "feed_title": feed_title,
                "episodes": episodes,
            })),
            DaemonRes::PodcastStatusRes { status } => Some(serde_json::json!({ "status": status })),
            DaemonRes::RadioStationsRes { stations } => {
                Some(serde_json::json!({ "stations": stations }))
            }
            DaemonRes::RadioTagsRes { tags } => Some(serde_json::json!({ "tags": tags })),
            DaemonRes::RadioCountriesRes { countries } => {
                Some(serde_json::json!({ "countries": countries }))
            }
            DaemonRes::ChartsSourcesRes { sources } => {
                Some(serde_json::json!({ "sources": sources }))
            }
            DaemonRes::ChartsListRes { charts } => Some(serde_json::json!({ "charts": charts })),
            DaemonRes::ChartsTracksRes { tracks } => Some(serde_json::json!({ "tracks": tracks })),
            DaemonRes::ChartsLoaded { charts } => Some(serde_json::json!({ "charts": charts })),
            DaemonRes::ChartTracksLoaded { tracks } => {
                Some(serde_json::json!({ "tracks": tracks }))
            }
            DaemonRes::CoverArt { data } => Some(serde_json::json!({ "data": data })),
            DaemonRes::SyncStatus {
                running,
                kind,
                synced,
                total,
            } => Some(serde_json::json!({
                "running": running,
                "kind": kind,
                "synced": synced,
                "total": total,
            })),
            DaemonRes::HealthReport { report } => Some(serde_json::json!({ "report": report })),
            DaemonRes::EqPresets { presets } => Some(serde_json::json!({ "presets": presets })),
            DaemonRes::LastfmAuthUrlRes { url } => Some(serde_json::json!({ "url": url })),
            DaemonRes::LastfmStatusRes {
                enabled,
                api_key,
                session_token,
                ready,
                loved,
            } => Some(serde_json::json!({
                "enabled": enabled,
                "api_key": api_key,
                "session_token": session_token,
                "ready": ready,
                "loved": loved,
            })),
            DaemonRes::YtDownloadProgress {
                id,
                url,
                title,
                progress,
                status,
                error,
                file_path,
                downloaded_bytes,
                total_bytes,
                rate_bps,
                eta_secs,
            } => Some(serde_json::json!({
                "id": id,
                "url": url,
                "title": title,
                "progress": progress,
                "status": status,
                "error": error,
                "file_path": file_path,
                "downloaded_bytes": downloaded_bytes,
                "total_bytes": total_bytes,
                "rate_bps": rate_bps,
                "eta_secs": eta_secs,
            })),
            DaemonRes::YtDownloadResult {
                id,
                url,
                title,
                file_path,
            } => Some(serde_json::json!({
                "id": id,
                "url": url,
                "title": title,
                "file_path": file_path,
            })),
            DaemonRes::Error { message } => return WireRes::err(id, message),
            DaemonRes::Pong => None,
        };

        WireRes::ok(id, data)
    }

    /// Serialize this typed response straight to the wire JSON line, in one
    /// pass, without building the intermediate `Value` tree that
    /// [`to_wire`](Self::to_wire) + `serde_json::to_string` requires. Keeps
    /// the exact same envelope shape (`{"id":N,"ok":true/"true"/...,fields}`)
    /// so the client's `from_wire` parses it identically.
    pub fn to_wire_line(self, id: u64) -> Result<String, serde_json::Error> {
        use std::fmt::Write as _;
        if let DaemonRes::Error { message } = &self {
            return Ok(format!(
                "{{\"id\":{id},\"ok\":false,\"error\":{}}}",
                serde_json::to_string(message)?
            ));
        }
        let mut line = format!("{{\"id\":{id},\"ok\":true");
        macro_rules! field {
            ($name:literal, $val:expr) => {{
                line.push(',');
                let _ = write!(line, "\"{}\":", $name);
                line.push_str(&serde_json::to_string($val)?);
            }};
        }
        macro_rules! flatten {
            ($val:expr) => {{
                line.push(',');
                let s = serde_json::to_string($val)?;
                // WireRes flattens an object Value into the envelope; strip
                // the object braces to mirror that behaviour. DaemonRes::Value
                // always holds an object in practice (e.g. {"volume": N}).
                if s.starts_with('{') && s.ends_with('}') {
                    line.push_str(&s[1..s.len() - 1]);
                } else {
                    line.push_str(&s);
                }
            }};
        }
        match self {
            DaemonRes::Ok | DaemonRes::Pong => {}
            DaemonRes::Value { value } => flatten!(&value),
            DaemonRes::Tracks { tracks } => field!("tracks", &tracks),
            DaemonRes::QueueState { queue, cursor } => {
                field!("queue", &queue);
                field!("cursor", &cursor);
            }
            DaemonRes::Status { state } => field!("state", &state),
            DaemonRes::Playlists { playlists } => field!("playlists", &playlists),
            DaemonRes::YtSearchResults { query, results } => {
                field!("query", &query);
                field!("results", &results);
            }
            DaemonRes::StreamInfo { info } => field!("info", &info),
            DaemonRes::Lyrics { lyrics } => field!("lyrics", &lyrics),
            DaemonRes::SpotifyStatusRes { status } => field!("status", &status),
            DaemonRes::SpotifyOauthStarted { url } => field!("url", &url),
            DaemonRes::SpotifyPlaylistsRes { playlists } => field!("playlists", &playlists),
            DaemonRes::SpotifyTracksRes { tracks } => field!("tracks", &tracks),
            DaemonRes::SpotifyImageRes { data } => field!("data", &data),
            DaemonRes::SubsonicStatusRes { status } => field!("status", &status),
            DaemonRes::SubsonicSearchRes { results } => field!("results", &results),
            DaemonRes::SubsonicAlbumsRes { albums } => field!("albums", &albums),
            DaemonRes::SubsonicTracksRes { tracks } => field!("tracks", &tracks),
            DaemonRes::SubsonicPingRes { message } => field!("message", &message),
            DaemonRes::PodcastFeedsRes { feeds } => field!("feeds", &feeds),
            DaemonRes::PodcastEpisodesRes {
                feed_id,
                feed_title,
                episodes,
            } => {
                field!("feed_id", &feed_id);
                field!("feed_title", &feed_title);
                field!("episodes", &episodes);
            }
            DaemonRes::PodcastStatusRes { status } => field!("status", &status),
            DaemonRes::RadioStationsRes { stations } => field!("stations", &stations),
            DaemonRes::RadioTagsRes { tags } => field!("tags", &tags),
            DaemonRes::RadioCountriesRes { countries } => field!("countries", &countries),
            DaemonRes::ChartsSourcesRes { sources } => field!("sources", &sources),
            DaemonRes::ChartsListRes { charts } => field!("charts", &charts),
            DaemonRes::ChartsTracksRes { tracks } => field!("tracks", &tracks),
            DaemonRes::ChartsLoaded { charts } => field!("charts", &charts),
            DaemonRes::ChartTracksLoaded { tracks } => field!("tracks", &tracks),
            DaemonRes::CoverArt { data } => field!("data", &data),
            DaemonRes::SyncStatus {
                running,
                kind,
                synced,
                total,
            } => {
                field!("running", &running);
                field!("kind", &kind);
                field!("synced", &synced);
                field!("total", &total);
            }
            DaemonRes::HealthReport { report } => field!("report", &report),
            DaemonRes::EqPresets { presets } => field!("presets", &presets),
            DaemonRes::LastfmAuthUrlRes { url } => field!("url", &url),
            DaemonRes::LastfmStatusRes {
                enabled,
                api_key,
                session_token,
                ready,
                loved,
            } => {
                field!("enabled", &enabled);
                field!("api_key", &api_key);
                field!("session_token", &session_token);
                field!("ready", &ready);
                field!("loved", &loved);
            }
            DaemonRes::YtDownloadProgress {
                id,
                url,
                title,
                progress,
                status,
                error,
                file_path,
                downloaded_bytes,
                total_bytes,
                rate_bps,
                eta_secs,
            } => {
                field!("id", &id);
                field!("url", &url);
                field!("title", &title);
                field!("progress", &progress);
                field!("status", &status);
                field!("error", &error);
                field!("file_path", &file_path);
                field!("downloaded_bytes", &downloaded_bytes);
                field!("total_bytes", &total_bytes);
                field!("rate_bps", &rate_bps);
                field!("eta_secs", &eta_secs);
            }
            DaemonRes::YtDownloadResult {
                id,
                url,
                title,
                file_path,
            } => {
                field!("id", &id);
                field!("url", &url);
                field!("title", &title);
                field!("file_path", &file_path);
            }
            // Handled by the early return above.
            DaemonRes::Error { .. } => unreachable!(),
        }
        line.push('}');
        Ok(line)
    }

    /// Reconstruct a typed `DaemonRes` from a parsed `WireRes` keyed by the
    /// `cmd` string of the request that produced it.
    pub fn from_wire(cmd: &str, wire: &WireRes) -> Self {
        match (wire.ok, &wire.error) {
            (Some(false), Some(msg)) => DaemonRes::Error {
                message: msg.clone(),
            },
            (Some(false), None) => DaemonRes::Error {
                message: "unknown error".into(),
            },
            (Some(true), _) => {
                let data = wire.data.clone().unwrap_or(Value::Null);
                Self::ok_from_data(cmd, data)
            }
            _ => DaemonRes::Error {
                message: "malformed response".into(),
            },
        }
    }

    fn ok_from_data(cmd: &str, data: Value) -> Self {
        let is_ack = match &data {
            Value::Null => true,
            Value::Object(map) => map.is_empty(),
            _ => false,
        };
        if is_ack {
            return if cmd == "ping" {
                DaemonRes::Pong
            } else {
                DaemonRes::Ok
            };
        }
        match cmd {
            "get_status" | "get_status_lite" => {
                match serde_json::from_value::<Box<DaemonState>>(field(&data, "state")) {
                    Ok(state) => DaemonRes::Status { state },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "queue" => {
                let cursor = data.get("cursor").and_then(|c| c.as_u64()).unwrap_or(0);
                match serde_json::from_value::<Vec<TrackInfo>>(field(&data, "queue")) {
                    Ok(queue) => DaemonRes::QueueState {
                        queue: Box::new(queue),
                        cursor,
                    },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "search" | "get_favourites" => {
                match serde_json::from_value::<Vec<TrackInfo>>(field(&data, "tracks")) {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "library" => {
                if data.get("running").is_some() {
                    #[derive(Deserialize)]
                    struct D {
                        running: bool,
                        kind: SyncKind,
                        synced: usize,
                        total: usize,
                    }
                    match serde_json::from_value::<D>(data.clone()) {
                        Ok(d) => DaemonRes::SyncStatus {
                            running: d.running,
                            kind: d.kind,
                            synced: d.synced,
                            total: d.total,
                        },
                        Err(_) => DaemonRes::Value { value: data },
                    }
                } else if data.get("playlists").is_some() {
                    match serde_json::from_value::<Vec<Playlist>>(field(&data, "playlists")) {
                        Ok(playlists) => DaemonRes::Playlists { playlists },
                        Err(_) => DaemonRes::Value { value: data },
                    }
                } else {
                    match serde_json::from_value::<Vec<TrackInfo>>(field(&data, "tracks")) {
                        Ok(tracks) => DaemonRes::Tracks {
                            tracks: Box::new(tracks),
                        },
                        Err(_) => DaemonRes::Value { value: data },
                    }
                }
            }
            "yt_search_poll" => {
                let query = field_str(&data, "query").to_string();
                match serde_json::from_value::<Vec<YTSearchResult>>(field(&data, "results")) {
                    Ok(results) => DaemonRes::YtSearchResults { query, results },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "yt_resolve_stream" => {
                match serde_json::from_value::<Box<StreamInfo>>(field(&data, "info")) {
                    Ok(info) => DaemonRes::StreamInfo { info },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "get_cover_art" | "artist_cover_art" => {
                match serde_json::from_value::<Option<String>>(field(&data, "data")) {
                    Ok(data) => DaemonRes::CoverArt { data },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "get_lyrics" => {
                match serde_json::from_value::<Option<LrcData>>(field(&data, "lyrics")) {
                    Ok(lyrics) => DaemonRes::Lyrics { lyrics },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_status" => {
                match serde_json::from_value::<SpotifyStatus>(field(&data, "status")) {
                    Ok(status) => DaemonRes::SpotifyStatusRes { status },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_playlists" => {
                match serde_json::from_value::<Vec<SpotifyPlaylist>>(field(&data, "playlists")) {
                    Ok(playlists) => DaemonRes::SpotifyPlaylistsRes { playlists },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_search_web"
            | "spotify_playlist_tracks"
            | "spotify_album_tracks"
            | "spotify_artist_top_tracks" => {
                match serde_json::from_value::<Vec<SpotifyTrack>>(field(&data, "tracks")) {
                    Ok(tracks) => DaemonRes::SpotifyTracksRes { tracks },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_set_token" | "spotify_clear" => {
                match serde_json::from_value::<SpotifyStatus>(field(&data, "status")) {
                    Ok(status) => DaemonRes::SpotifyStatusRes { status },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_track_image" => {
                match serde_json::from_value::<Option<String>>(field(&data, "data")) {
                    Ok(data) => DaemonRes::SpotifyImageRes { data },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "spotify_oauth_start" => DaemonRes::SpotifyOauthStarted {
                url: field_str(&data, "url").to_string(),
            },
            "subsonic_status" => {
                match serde_json::from_value::<SubsonicStatus>(field(&data, "status")) {
                    Ok(status) => DaemonRes::SubsonicStatusRes { status },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "subsonic_search" => {
                match serde_json::from_value::<SubsonicSearchResults>(field(&data, "results")) {
                    Ok(results) => DaemonRes::SubsonicSearchRes { results },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "subsonic_albums" => {
                match serde_json::from_value::<Vec<SubsonicAlbum>>(field(&data, "albums")) {
                    Ok(albums) => DaemonRes::SubsonicAlbumsRes { albums },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "subsonic_album_tracks" => {
                match serde_json::from_value::<Vec<SubsonicTrack>>(field(&data, "tracks")) {
                    Ok(tracks) => DaemonRes::SubsonicTracksRes { tracks },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "subsonic_ping" => DaemonRes::SubsonicPingRes {
                message: field_str(&data, "message").to_string(),
            },
            "subsonic_cover" => {
                match serde_json::from_value::<Option<String>>(field(&data, "data")) {
                    Ok(data) => DaemonRes::SpotifyImageRes { data },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "podcast_feeds" => {
                match serde_json::from_value::<Vec<PodcastFeed>>(field(&data, "feeds")) {
                    Ok(feeds) => DaemonRes::PodcastFeedsRes { feeds },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "podcast_episodes" => {
                let feed_id = field_str(&data, "feed_id").to_string();
                let feed_title = field_str(&data, "feed_title").to_string();
                match serde_json::from_value::<Vec<PodcastEpisode>>(field(&data, "episodes")) {
                    Ok(episodes) => DaemonRes::PodcastEpisodesRes {
                        feed_id,
                        feed_title,
                        episodes,
                    },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "podcast_status" => {
                match serde_json::from_value::<PodcastStatus>(field(&data, "status")) {
                    Ok(status) => DaemonRes::PodcastStatusRes { status },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "radio_search" | "radio_top" | "radio_bytag" | "radio_bycountry" => {
                match serde_json::from_value::<Vec<RadioStation>>(field(&data, "stations")) {
                    Ok(stations) => DaemonRes::RadioStationsRes { stations },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "radio_tags" => match serde_json::from_value::<Vec<RadioTag>>(field(&data, "tags")) {
                Ok(tags) => DaemonRes::RadioTagsRes { tags },
                Err(_) => DaemonRes::Value { value: data },
            },
            "radio_countries" => {
                match serde_json::from_value::<Vec<RadioCountry>>(field(&data, "countries")) {
                    Ok(countries) => DaemonRes::RadioCountriesRes { countries },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "check_health" => {
                match serde_json::from_value::<Box<HealthReport>>(field(&data, "report")) {
                    Ok(report) => DaemonRes::HealthReport { report },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "lastfm_auth_url" => DaemonRes::LastfmAuthUrlRes {
                url: field_str(&data, "url").to_string(),
            },
            "lastfm_status" => DaemonRes::LastfmStatusRes {
                enabled: data
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                api_key: data
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                session_token: data
                    .get("session_token")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                ready: data.get("ready").and_then(|v| v.as_bool()).unwrap_or(false),
                loved: data.get("loved").and_then(|v| v.as_bool()).unwrap_or(false),
            },
            "yt_download_progress" => DaemonRes::YtDownloadProgress {
                id: data.get("id").and_then(|v| v.as_u64()).unwrap_or(0),
                url: field_str(&data, "url").to_string(),
                title: field_str(&data, "title").to_string(),
                progress: data.get("progress").and_then(|v| v.as_f64()).unwrap_or(0.0),
                status: field_str(&data, "status").to_string(),
                error: data.get("error").and_then(|v| v.as_str()).map(String::from),
                file_path: data
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                downloaded_bytes: data.get("downloaded_bytes").and_then(|v| v.as_u64()),
                total_bytes: data.get("total_bytes").and_then(|v| v.as_u64()),
                rate_bps: data.get("rate_bps").and_then(|v| v.as_f64()),
                eta_secs: data.get("eta_secs").and_then(|v| v.as_u64()),
            },
            "yt_download_result" => DaemonRes::YtDownloadResult {
                id: data.get("id").and_then(|v| v.as_u64()).unwrap_or(0),
                url: field_str(&data, "url").to_string(),
                title: field_str(&data, "title").to_string(),
                file_path: field_str(&data, "file_path").to_string(),
            },
            "list_eq_presets" => {
                match serde_json::from_value::<Vec<String>>(field(&data, "presets")) {
                    Ok(presets) => DaemonRes::EqPresets { presets },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "ping" => DaemonRes::Pong,
            _ => {
                if data.is_null() {
                    DaemonRes::Ok
                } else {
                    DaemonRes::Value { value: data }
                }
            }
        }
    }
}

/// Health status of a daemon component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub uptime_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
    Error,
}

/// Diagnostic report from the daemon, similar to Neovim's `:checkhealth`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub daemon_uptime_secs: f64,
    pub version: String,
    pub components: Vec<ComponentHealth>,
}
