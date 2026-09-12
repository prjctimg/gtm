// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Application state machine: input handling, IPC dispatch, crossfade
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use gtm_core::client::{DaemonClient, LastfmStatus};
use gtm_core::global::{DaemonState, EqPreset, PlaybackStatus, RepeatMode};
use gtm_core::ipc::{CacheKind, DaemonEvent, DaemonRes, HealthReport, SyncKind};
use gtm_core::podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};
use gtm_core::radio::{RadioCountry, RadioStation, RadioTag};
use gtm_core::secret::{SPOTIFY_CLIENT_ID_KEY, get_secret, set_secret};
use gtm_core::spotify::{LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use gtm_core::state::{ThemeMode, TrackSort};
use gtm_core::subsonic::{SubsonicAlbum, SubsonicSearchResults, SubsonicStatus, SubsonicTrack};
use gtm_core::track::{LrcData, LrcLine, Playlist, TrackInfo, YTSearchResult};
use gtm_core::{CoreError, MAX_SPEED, MAX_VOLUME, MIN_SPEED, MetadataPatch};
use ratatui::Terminal;
use ratatui::layout::Alignment;
use ratatui::widgets::Paragraph;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tachyonfx::EffectManager;
use tokio::sync::mpsc;

use base64::Engine;

use crate::footer::{FooterCache, FooterPreset, merged_presets};
use crate::keymap::{
    BoundCommand, KeyContext, Keybindings, KeyboardAction, default_keybindings, detect_clashes,
    parse_key_event,
};
use crate::mouse::{MouseMap, MouseZone};
use crate::oauth::capture_lastfm_token_loopback;
use crate::picker::{PickerId, PickerManager, PickerSource};
use crate::progress::{ProgressSmoother, ProgressStyle};
use crate::reactive::{ReactivePalette, derive_theme, extract_palette};
use crate::theme::{AppTheme, ThemeEntry, blend_colors, chadrula, detect_os_theme, merged_themes};
use crate::ui;
use crate::ui::{
    CROSSFADE_DURATIONS, Command, CommandPalette, HELP_LINES, cover_provider_label,
    theme_mode_label,
};
use crate::visualizer::{AudioVisualizer, VisualizerPreset};

fn prefs_path() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".config")
        });
    let dir = config.join("gtm");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.toml")
}

/// Return the config file path, creating the config directory and a
/// default (pretty-printed) `config.toml` if it does not exist yet. Used by
/// the `gtm config` CLI command to give the editor a valid starting point.
pub(crate) fn ensure_prefs_file() -> std::path::PathBuf {
    let path = prefs_path();
    if !path.exists()
        && let Ok(toml_s) = toml::to_string_pretty(&Prefs::default())
    {
        let _ = std::fs::write(&path, format!("{toml_s}\n"));
    }
    path
}

/// Ordered list of all equalizer presets (order matches the picker).
const EQ_PRESETS: [EqPreset; 16] = [
    EqPreset::Flat,
    EqPreset::Normal,
    EqPreset::Pop,
    EqPreset::Rock,
    EqPreset::Jazz,
    EqPreset::Classical,
    EqPreset::Bass,
    EqPreset::Vocal,
    EqPreset::Electronic,
    EqPreset::HipHop,
    EqPreset::Latin,
    EqPreset::Acoustic,
    EqPreset::Podcast,
    EqPreset::Dance,
    EqPreset::Headphones,
    EqPreset::Speaker,
];

fn default_auto_fetch_lyrics() -> bool {
    true
}

fn default_icon_style() -> String {
    "mdi".to_string()
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Prefs {
    #[serde(default = "default_theme_name")]
    theme_name: String,
    #[serde(default)]
    transparent_bg: bool,
    #[serde(default)]
    transparent_pickers: bool,
    #[serde(default)]
    reactive_theme: bool,
    #[serde(default = "default_reactive_intensity")]
    reactive_theme_intensity: f32,
    #[serde(default = "default_footer_preset_name")]
    footer_preset_name: String,
    #[serde(default)]
    progress_style: ProgressStyle,
    #[serde(default)]
    visualizer_preset: VisualizerPreset,
    #[serde(default = "default_time_format")]
    time_format: String,
    #[serde(default = "default_theme_mode")]
    theme_mode: String,
    #[serde(default = "default_track_sort")]
    track_sort: TrackSort,
    #[serde(default)]
    keybindings: std::collections::HashMap<String, String>,
    #[serde(default = "default_notification_modes")]
    notification_modes: std::collections::HashMap<String, String>,
    #[serde(default = "default_cover_provider")]
    cover_provider: String,
    #[serde(default = "default_auto_fetch_lyrics")]
    auto_fetch_lyrics: bool,
    #[serde(default = "default_icon_style")]
    icon_style: String,
    #[serde(default)]
    hide_footer: bool,
}

fn default_cover_provider() -> String {
    "auto".into()
}

fn default_theme_name() -> String {
    "Chadrula".into()
}

/// Default reactive background wash strength (matches the pre-0.2.84
/// hardcoded blend factor in `reactive::derive_theme`).
fn default_reactive_intensity() -> f32 {
    0.34
}

fn default_track_sort() -> TrackSort {
    TrackSort::Recents
}

fn default_time_format() -> String {
    "%H:%M".into()
}

fn default_theme_mode() -> String {
    "auto".into()
}

/// Resolve which theme to start with. Honors the user's persisted
/// `theme_name`, unless `theme_mode` is "auto" (the default) and the OS can be
/// queried for its dark/light preference — in which case a theme matching the
/// OS preference is chosen over the saved name. A saved name still wins when
/// it already agrees with the OS preference.
fn resolve_theme_index(themes: &[ThemeEntry], theme_name: &str, mode: &str) -> usize {
    if themes.is_empty() {
        return 0;
    }
    // Only override with the OS preference when in auto mode.
    let os = if mode == "auto" {
        detect_os_theme()
    } else {
        match mode {
            "dark" => Some(ThemeMode::Dark),
            "light" => Some(ThemeMode::Light),
            _ => None,
        }
    };
    if let Some(os) = os {
        let os_light = os == ThemeMode::Light;
        // Prefer an exact match on the persisted name if it agrees with the OS.
        if let Some(idx) = themes.iter().position(|t| t.name == theme_name)
            && themes[idx].light == os_light
        {
            return idx;
        }
        // Otherwise pick the first theme whose light flag matches the OS.
        if let Some(idx) = themes.iter().position(|t| t.light == os_light) {
            return idx;
        }
    }
    themes
        .iter()
        .position(|t| t.name == theme_name)
        .unwrap_or(0)
}

fn default_footer_preset_name() -> String {
    "Default".into()
}

/// Default per-type notification modes: everything Floating.
fn default_notification_modes() -> std::collections::HashMap<String, String> {
    NotifType::ALL
        .iter()
        .map(|t| {
            (
                t.as_str().to_string(),
                NotifMode::Floating.as_str().to_string(),
            )
        })
        .collect()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme_name: default_theme_name(),
            transparent_bg: false,
            transparent_pickers: false,
            reactive_theme: false,
            reactive_theme_intensity: default_reactive_intensity(),
            footer_preset_name: default_footer_preset_name(),
            progress_style: ProgressStyle::default(),
            visualizer_preset: VisualizerPreset::default(),
            time_format: default_time_format(),
            theme_mode: default_theme_mode(),
            track_sort: default_track_sort(),
            keybindings: std::collections::HashMap::new(),
            notification_modes: default_notification_modes(),
            cover_provider: default_cover_provider(),
            auto_fetch_lyrics: default_auto_fetch_lyrics(),
            icon_style: default_icon_style(),
            hide_footer: false,
        }
    }
}

fn load_prefs() -> Prefs {
    let path = prefs_path();
    let Ok(s) = std::fs::read_to_string(&path) else {
        return Prefs::default();
    };
    toml::from_str::<Prefs>(&s).unwrap_or_default()
}

fn save_prefs(prefs: &Prefs) {
    if let Ok(s) = toml::to_string(prefs) {
        let _ = std::fs::write(prefs_path(), s);
    }
}

fn build_keybindings(overrides: &std::collections::HashMap<String, String>) -> Keybindings {
    let mut defaults = default_keybindings();

    if overrides.is_empty() {
        return defaults;
    }

    // Parse user overrides into (KeyEvent, action_name, contexts) triples.
    let mut user_bindings: Vec<(crossterm::event::KeyEvent, String, Vec<KeyContext>)> = Vec::new();
    for (key_str, action_str) in overrides {
        let key = match parse_key_event(key_str) {
            Some(k) => k,
            None => {
                eprintln!("gtm: unknown key \"{}\" in config keybindings", key_str);
                continue;
            }
        };
        let action = match KeyboardAction::from_name(action_str) {
            Some(a) => a,
            None => {
                eprintln!(
                    "gtm: unknown action \"{}\" in config keybindings",
                    action_str
                );
                continue;
            }
        };
        // Bind in Normal + List context by default for most actions.
        let contexts = vec![KeyContext::Normal, KeyContext::List];
        user_bindings.push((key, action_str.clone(), contexts));
        defaults.bindings.push((
            key,
            BoundCommand {
                action,
                contexts: vec![KeyContext::Normal, KeyContext::List],
            },
        ));
    }

    let warnings = detect_clashes(&user_bindings);
    for w in &warnings {
        eprintln!("gtm: keybinding clash: {}", w);
    }

    defaults
}

pub const NUM_SETTINGS_CATEGORIES: usize = 4;
pub const LIBRARY_CATEGORIES: &[&str] = &[
    "All Tracks",
    "Liked",
    "Albums",
    "Artists",
    "Playlists",
    "Spotify",
];

/// Returns true if the terminal doesn't support image protocols (Neovim, Zellij, etc.).
pub fn no_image_protocol() -> bool {
    std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok()
}

/// Kind of item the library track-info block is currently describing.  The
/// widget is context aware of the active list type: tracks show
/// title / artist / album / duration, albums show album + artist + count,
/// artists show the artist + count, and playlist rows show the playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackInfoKind {
    Track,
    Album,
    Artist,
    Playlist,
    SpotifyPlaylist,
    /// Drill-down row in a Spotify playlist (no local cover available).
    SpotifyTrack,
}

pub enum InputMode {
    Normal,
    Searching,
}

#[derive(Debug, Clone)]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Category of a notification toast. Each category has an independent
/// visibility mode (`NotifMode`), letting the user keep e.g. playback STATUS
/// toasts quiet while errors stay on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NotifType {
    Playback,
    Prefs,
    NowPlaying,
    Library,
    Downloads,
    Spotify,
    Subsonic,
    Podcast,
    Radio,
    Lastfm,
    System,
}

impl NotifType {
    pub const ALL: [NotifType; 11] = [
        NotifType::Playback,
        NotifType::Prefs,
        NotifType::NowPlaying,
        NotifType::Library,
        NotifType::Downloads,
        NotifType::Spotify,
        NotifType::Subsonic,
        NotifType::Podcast,
        NotifType::Radio,
        NotifType::Lastfm,
        NotifType::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NotifType::Playback => "Playback",
            NotifType::Prefs => "Prefs / UI",
            NotifType::NowPlaying => "Now Playing / Queue",
            NotifType::Library => "Library",
            NotifType::Downloads => "YouTube / Downloads",
            NotifType::Spotify => "Spotify",
            NotifType::Subsonic => "Subsonic",
            NotifType::Podcast => "Podcast",
            NotifType::Radio => "Radio",
            NotifType::Lastfm => "Last.fm",
            NotifType::System => "System / Errors",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NotifType::Playback => "playback",
            NotifType::Prefs => "prefs",
            NotifType::NowPlaying => "nowplaying",
            NotifType::Library => "library",
            NotifType::Downloads => "downloads",
            NotifType::Spotify => "spotify",
            NotifType::Subsonic => "subsonic",
            NotifType::Podcast => "podcast",
            NotifType::Radio => "radio",
            NotifType::Lastfm => "lastfm",
            NotifType::System => "system",
        }
    }

    pub fn from_str_lossy(s: &str) -> NotifType {
        match s {
            "playback" => NotifType::Playback,
            "prefs" => NotifType::Prefs,
            "nowplaying" => NotifType::NowPlaying,
            "library" => NotifType::Library,
            "downloads" => NotifType::Downloads,
            "spotify" => NotifType::Spotify,
            "subsonic" => NotifType::Subsonic,
            "podcast" => NotifType::Podcast,
            "radio" => NotifType::Radio,
            "lastfm" => NotifType::Lastfm,
            _ => NotifType::System,
        }
    }
}

/// How a notification category is surfaced: floating toast, terse footer
/// one-liner, or suppressed (history only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotifMode {
    Floating,
    Footer,
    Off,
}

impl NotifMode {
    pub const ALL: [NotifMode; 3] = [NotifMode::Floating, NotifMode::Footer, NotifMode::Off];

    pub fn label(self) -> &'static str {
        match self {
            NotifMode::Floating => "Floating",
            NotifMode::Footer => "Footer",
            NotifMode::Off => "Off",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NotifMode::Floating => "floating",
            NotifMode::Footer => "footer",
            NotifMode::Off => "off",
        }
    }

    pub fn from_str_lossy(s: &str) -> NotifMode {
        match s {
            "footer" => NotifMode::Footer,
            "off" => NotifMode::Off,
            _ => NotifMode::Floating,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideDirection {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub message: String,
    pub kind: NotificationKind,
    pub expires_at: std::time::Instant,
    pub slide_direction: SlideDirection,
    pub is_volume: bool,
    pub volume_value: u8,
    pub animation_progress: f32,
    pub trivial: bool,
}

#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub title: String,
    pub message: String,
    pub kind: NotificationKind,
    pub at: std::time::Instant,
}

/// Live state of one daemon-side yt-dlp download, for the footer Download
/// module. Percent is an EMA of the yt-dlp values so the bar glides instead of
/// jittering between updates.
#[derive(Debug, Clone)]
pub struct DownloadProgressView {
    pub url: String,
    pub title: String,
    pub status: String,
    pub file_path: Option<String>,
    pub percent: f64,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub rate_bytes_per_sec: Option<f64>,
    pub eta_secs: Option<u64>,
    pub updated_at: std::time::Instant,
}

pub struct UpNextNotif {
    pub track: TrackInfo,
    pub cover: Option<Vec<u8>>,
    pub cover_stateful: Option<StatefulProtocol>,
    pub started_at: std::time::Instant,
    pub total_secs: f64,
    pub cover_fetch_id: Option<i64>,
    pub cover_fetch_gen: Option<u64>,
}

/// One row in the SearchLibrary fuzzy-finder, resolved from a `PickerSource`.
#[derive(Debug, Clone)]
pub enum LibraryPick {
    Track(usize),
    Artist(String),
    Album(String),
    Playlist(usize),
}

pub struct SleepTimerState {
    pub remaining: Option<u64>,
    pub minutes: u32,
    pub input_mode: bool,
    pub input_buf: String,
}

pub struct MetadataEditState {
    pub edit_track_id: Option<i64>,
    pub fields: [String; 7],
    pub field_idx: usize,
    pub cover: Option<Vec<u8>>,
    pub cover_stateful: Option<StatefulProtocol>,
    pub cover_dirty: bool,
    pub cover_fetch_gen: Option<u64>,
}

pub struct NowPlayingCoverState {
    pub image: Option<Vec<u8>>,
    pub track_id: Option<i64>,
    pub track_path: Option<String>,
    pub picker: Option<Picker>,
    pub stateful: Option<StatefulProtocol>,
    pub pending_gen: Option<u64>,
}

/// A pending prompt that waits for user input. Used for confirmations and
/// other modal dialogs that should persist until a specific key is pressed.
pub struct PendingPrompt {
    pub message: String,
    /// Keys that confirm the action (e.g., 'y', 'Y', Enter)
    pub confirm_keys: Vec<KeyCode>,
    /// Keys that cancel the action (e.g., 'n', 'N', 'q', Esc)
    pub cancel_keys: Vec<KeyCode>,
    /// Type of prompt to handle the action
    pub prompt_type: PromptType,
}

#[derive(Debug, Clone, Copy)]
pub enum PromptType {
    DeleteTrack(i64),
    MultiselectDelete(usize),
    MultiselectAddToQueue,
    MultiselectAddToPlaylist,
    None,
}

/// Spotify search/link UI state, grouped under `App::spotify`.
pub struct SpotifyView {
    pub status: Option<SpotifyStatus>,
    pub playlists: Vec<SpotifyPlaylist>,
    pub playlist_tracks_cache: Vec<SpotifyTrack>,
    pub search_results: Vec<(String, String, SpotifyTrack)>,
    pub link_input: String,
    pub token_input: String,
    /// True while the OAuth browser flow is pending; the SpotifyLink picker
    /// shows a "waiting for you to finish login" state until linked.
    pub oauth_pending: bool,
    /// Authorize URL of the in-flight OAuth flow, shown in the SpotifyLink
    /// picker so the user can copy it even when no browser can be opened.
    pub oauth_url: Option<String>,
    /// Error from the most recent OAuth attempt, shown in the picker.
    pub oauth_error: Option<String>,
    /// Local redirect port for the Spotify OAuth flow (editable in the picker).
    pub oauth_port: String,
    /// Active field in the SpotifyLink picker (0 = client id, 1 = port).
    pub link_field: usize,
    pub search_debounce: Option<std::time::Instant>,
    pub web_seq: u64,
    /// Cover art for the SpotifySearch picker preview window, fetched from the
    /// album-cover URL of the highlighted web result.
    pub preview_cover: Option<Vec<u8>>,
    pub preview_cover_stateful: Option<StatefulProtocol>,
    pub last_preview_fetch: Option<String>,
    pub last_preview_fetch_gen: Option<u64>,
}

/// Subsonic (Navidrome) picker state, grouped under `App::subsonic`.
#[derive(Default)]
pub struct SubsonicView {
    pub status: Option<SubsonicStatus>,
    /// Results of the last server search (flattened into an ordered row list).
    pub search_results: SubsonicSearchResults,
    pub search_pending: bool,
    pub albums: Vec<SubsonicAlbum>,
    pub albums_pending: bool,
    /// Track list of the album currently drilled into (`selected_album`).
    pub album_tracks: Vec<SubsonicTrack>,
    pub selected_album: Option<SubsonicAlbum>,
    /// Cover base64 preview of the highlighted Subsonic track row.
    pub cover_preview: Option<String>,
    /// Track id whose cover was requested (in-flight marker: the daemon replies
    /// with the base64 payload which replaces `cover_preview`).
    pub cover_track_id: Option<String>,
    /// Fields for the SubsonicSetup form.
    pub form_server: String,
    pub form_user: String,
    pub form_password: String,
    pub form_focus: usize,
}

/// `gtm setup` wizard state, grouped under `App::setup`.
#[derive(Default)]
pub struct SetupView {
    /// Currently highlighted service in the Setup chooser (0..=2).
    pub selection: usize,
    /// Last.fm form fields (masked while typing) and flow state.
    pub lastfm_api_key: String,
    pub lastfm_api_secret: String,
    pub lastfm_focus: usize,
    /// True while waiting for the loopback callback after the browser opened.
    pub lastfm_pending: bool,
    pub lastfm_status: Option<LastfmStatus>,
    /// Authorization URL for manual copy when no browser can be opened.
    pub lastfm_auth_url: Option<String>,
    pub lastfm_error: Option<String>,
}

/// Selected row of the `gtm setup` service chooser.
pub fn setup_selection(app: &App) -> (usize, &'static str) {
    let names = ["spotify", "lastfm", "subsonic"];
    let sel = app.setup.selection.min(2);
    (sel, names[sel])
}

/// Podcast picker state, grouped under `App::podcast`.
#[derive(Default)]
pub struct PodcastView {
    pub status: Option<PodcastStatus>,
    pub feeds: Vec<PodcastFeed>,
    pub feeds_pending: bool,
    /// Episode list of the feed currently drilled into.
    pub episodes: Vec<PodcastEpisode>,
    pub episodes_feed_id: Option<String>,
    /// Draft feed URL for the PodcastSubscribe form.
    pub subscribe_url: String,
}

/// Which directory list the Radio Browse picker is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioBrowseKind {
    #[default]
    Tags,
    Countries,
}

/// Radio Browser picker state, grouped under `App::radio`.
#[derive(Default)]
pub struct RadioView {
    pub search: Vec<RadioStation>,
    pub search_pending: bool,
    pub top: Vec<RadioStation>,
    pub top_pending: bool,
    /// Tag list (RadioBrowseList in `Tags` kind).
    pub browse_tags: Vec<RadioTag>,
    /// Country list (RadioBrowseList in `Countries` kind).
    pub browse_countries: Vec<RadioCountry>,
    pub browse_pending: bool,
    /// Which list `RadioBrowseList` is showing.
    pub browse_kind: RadioBrowseKind,
    /// Tag/country selected at `RadioBrowseList`; stations live in
    /// `browse_stations`.
    pub browse_topic: String,
    pub browse_stations: Vec<RadioStation>,
    pub browse_stations_pending: bool,
}

/// Queue picker/view UI state, grouped under `App::queue`. Note this mirrors
/// (but is distinct from) the daemon-side `DaemonState::queue`.
pub struct QueueView {
    pub cache: Vec<TrackInfo>,
    pub cursor: usize,
    /// Queue move mode state: index of item being moved.
    pub move_index: Option<usize>,
    /// Target position in queue for move operation.
    pub move_target: usize,
    /// Cover art for the queue picker "Up Next" strip: fetched for the
    /// track after the current one, including locally-inserted (`id == 0`)
    /// entries.
    pub preview_cover: Option<Vec<u8>>,
    pub preview_cover_stateful: Option<StatefulProtocol>,
    pub last_preview_cover_fetch_id: Option<i64>,
    pub last_preview_cover_fetch_gen: Option<u64>,
    pub preview_cover_fail_until: Option<(i64, std::time::Instant)>,
}

/// Lyrics pane UI state, grouped under `App::lyrics`.
pub struct LyricsView {
    pub current: Option<LrcData>,
    pub scroll: usize,
    pub fetching: bool,
    /// Gen of the in-flight lyrics fetch; stale responses (track changed while
    /// a fetch was pending) are dropped when they don't match this.
    pub pending_gen: Option<u64>,
    /// Monotonic generation counter for lyrics fetches, disambiguates stale
    /// responses on fast track skips (mirrors `next_cover_gen`).
    pub next_gen: u64,
    pub show: bool,
    /// Whether the lyrics pane holds focus. While true, MoveUp/Down,
    /// PageUp/Down, Top/Bottom scroll the lyrics and take over from the
    /// time-sync driver until focus is released.
    pub pane_focus: bool,
    pub manual_scroll: bool,
}

pub struct App {
    pub theme: AppTheme,
    pub themes: Vec<ThemeEntry>,
    pub client: DaemonClient,
    pub state: DaemonState,
    pub display_position: f64,
    last_display_position: f64,
    /// Raw (guarded, un-smoothed) daemon playback position used for
    /// time-synced lyric matching so the active verse updates without the
    /// EMA lag that smooths the progress bar.
    raw_position: f64,
    /// Set when a seek is issued so the monotonic position guard is skipped
    /// (a backward seek would otherwise be clamped and never re-sync the lyric
    /// highlight). Cleared shortly after the seek lands.
    seek_pending: Option<std::time::Instant>,
    /// Coalesced seek commands: while the user holds a seek key (long-press),
    /// each repeat press adjusts `seek_cmd_accum` and the local position only.
    /// The daemon receives a single, debounced seek (via `ensure_seek_flush`)
    /// once the repeats settle — so it never does a full re-decode per keypress,
    /// which is what surfaced errors on long-press seeking.
    seek_cmd_accum: Option<f64>,
    last_seek_press: Option<std::time::Instant>,
    pub progress_smoother: ProgressSmoother,
    last_frame: std::time::Instant,
    pub frame_count: u64,
    /// Progress whip scanner position (Knight Rider style).
    pub scanner_pos: i32,
    /// Scanner direction: 1 = forward, -1 = backward.
    pub scanner_dir: i32,
    /// Hold frames at each end before reversing.
    pub scanner_hold: i32,
    /// Cursor blink toggle for search pickers (alternates every ~8 frames).
    pub cursor_blink: bool,
    pub input_mode: InputMode,
    pub search_query: String,
    /// Per-category selection index, keyed by `library_category`, so every
    /// list (All Tracks / Liked / Albums / Artists / Playlists / Spotify)
    /// keeps its own highlighted row.
    scroll_offset: [usize; LIBRARY_CATEGORIES.len()],
    pub library_category: usize,
    pub library_pane_focus: bool,
    pub settings_category: usize,
    pub settings_pane_focus: bool,
    pub settings_option: usize,
    pub tracks_cache: Vec<TrackInfo>,
    pub queue: QueueView,
    pub browse_detail: Option<String>,
    pub yt_results_cache: Vec<YTSearchResult>,
    pub playlist_cache: Vec<Playlist>,
    pub playlist_tracks_cache: Vec<TrackInfo>,
    pub spotify: SpotifyView,
    pub subsonic: SubsonicView,
    pub setup: SetupView,
    pub podcast: PodcastView,
    pub radio: RadioView,
    pub cookie_file: Option<String>,
    pub notifications: Vec<Notification>,
    pub notification_history: Vec<NotificationRecord>,
    pub footer_notification: Option<(String, std::time::Instant)>,
    /// Per-category notification visibility, hydrated from `Prefs` on config
    /// load and persisted via `current_prefs`.
    pub notification_modes: std::collections::HashMap<NotifType, NotifMode>,
    /// Cover provider preference (`auto`/`deezer`/`musicbrainz`/`spotify`),
    /// persisted in config.toml and consumed by the daemon for cover lookups.
    pub cover_provider: String,
    /// Whether to automatically fetch lyrics on track change
    pub auto_fetch_lyrics: bool,
    /// Icon style for command palette: "mdi" (Material Design Icons) or "emoji"
    pub icon_style: String,
    pub crossfade_duration: u8,
    pub yt_search_loading: bool,
    pub yt_search_debounce: Option<std::time::Instant>,
    pub yt_search_poll_deadline: Option<std::time::Instant>,
    /// Live download progress (keyed by daemon download id), surfaced in the
    /// footer Download module.
    pub downloads: std::collections::HashMap<u64, DownloadProgressView>,
    pub pending_delete: Option<(i64, String)>,
    /// Pending prompt for confirmations that require user input
    pub pending_prompt: Option<PendingPrompt>,
    pub pickers: PickerManager,
    pub sleep_timer: SleepTimerState,
    pub np_cover: NowPlayingCoverState,
    pub terminal_cols: u16,
    pub terminal_rows: u16,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    high_pri_cmd_rx: mpsc::UnboundedReceiver<TuiCommand>,
    high_pri_cmd_tx: mpsc::UnboundedSender<TuiCommand>,
    ipc_rx: mpsc::UnboundedReceiver<IpcResult>,
    ipc_tx: mpsc::UnboundedSender<IpcResult>,
    keybindings: Keybindings,
    prefs_keybindings: std::collections::HashMap<String, String>,
    pub theme_index: usize,
    pub list_scroll: usize,
    pub viewport_items: usize,
    pub transparent_bg: bool,
    pub transparent_pickers: bool,
    pub reactive_theme: bool,
    pub reactive_theme_intensity: f32,
    reactive_palette: Option<ReactivePalette>,
    pub last_action_name: Option<(String, std::time::Instant)>,
    pub footer_title_scroll: usize,
    /// strftime-style format string for the footer `Time` module.
    pub footer_time_format: String,
    /// OS-theme compliance mode: "auto" (detect dark/light), "dark", "light".
    pub theme_mode: String,
    /// How the library track list is sorted.
    pub track_sort: TrackSort,
    pub is_ready: bool,
    last_queue_cursor: u64,
    /// Set when the user manually triggers Next/Prev so the "Up next"
    /// notification only appears on genuine auto-advance.
    pub manual_track_advance: bool,
    /// True for the current track change if it was automatic (not a manual
    /// Next/Prev/seek). Captured when PlaybackStarted is drained, before the
    /// manual-advance flag is reset, so the dust animation can be gated to
    /// genuine auto-advances only.
    pub auto_track_advance: bool,
    last_track_path_display: Option<String>,
    prev_track_id: Option<i64>,
    prev_status: PlaybackStatus,
    prev_volume: u8,
    prev_cover_id: Option<i64>,
    cover_art_dirty: bool,
    pub footer_cache: FooterCache,
    pub footer_presets: Vec<FooterPreset>,
    pub footer_preset: usize,
    last_event_time: std::time::Instant,
    pub multiselect_mode: bool,
    pub progress_style: ProgressStyle,
    pub visualizer: AudioVisualizer,
    pub selected_indices: std::collections::HashSet<usize>,
    pending_motion: Option<char>,
    pub pending_playlist_track_ids: Vec<i64>,
    /// Id of a freshly-created playlist awaiting track selection.
    pub pending_playlist_id: Option<i64>,
    /// Tracks currently highlighted for the in-flight new-playlist flow.
    pub selected_playlist_track_ids: std::collections::HashSet<i64>,
    pub playlist_creating: bool,
    pub metadata: MetadataEditState,
    pub pending_quit: bool,
    /// Clickable row rectangles rebuilt every frame by `ui::render`
    ///.
    pub mouse_map: MouseMap,
    pub np_title_scroll: usize,
    /// Set on the first frame and on each track change; the render layer
    /// (re)starts the library/Now-Playing evolve animation once per trigger.
    pub track_anim_trigger: bool,
    /// Tachyonfx effect manager carrying the running evolve animation; kept
    /// alive across refresh frames until the effect completes.
    pub anim_fx: EffectManager<&'static str>,
    pub track_popup_visible: bool,
    pub track_popup_track_id: Option<i64>,
    pub track_popup_cover: Option<Vec<u8>>,
    pub popup_cover_stateful: Option<StatefulProtocol>,
    last_popup_cover_fetch_id: Option<i64>,
    last_popup_cover_fetch_gen: Option<u64>,
    /// Cover art for the SearchLibrary picker preview window.
    pub picker_preview_cover: Option<Vec<u8>>,
    pub picker_preview_stateful: Option<StatefulProtocol>,
    last_picker_preview_fetch_id: Option<i64>,
    last_picker_preview_fetch_gen: Option<u64>,
    /// Cover art for artist selections in the search picker preview.
    pub artist_cover: Option<Vec<u8>>,
    pub artist_cover_stateful: Option<StatefulProtocol>,
    last_artist_cover_fetch: Option<String>,
    last_artist_cover_fetch_gen: Option<u64>,
    /// Active "Up Next" crossfade-countdown notification.
    pub upnext: Option<UpNextNotif>,
    // Monotonic generation counter for all cover fetches — disambiguates
    // stale responses and `id == 0` reuse across different tracks.
    next_cover_gen: u64,
    pub lyrics: LyricsView,
    pub show_health_panel: bool,
    pub show_health_on_report: bool,
    pub health_report: Option<HealthReport>,
    pub hide_help_bar: bool,
    pub hide_footer: bool,
    pub pending_suspend: bool,
    last_config_mtime: Option<std::time::SystemTime>,
}

enum IpcResult {
    RefreshDone(Box<DaemonState>, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>, u64),
    PopupCoverArt(Option<Vec<u8>>, i64, u64),
    UpNextCover(Option<Vec<u8>>, i64, u64),
    QueuePreviewCover(Option<Vec<u8>>, i64, u64),
    PickerPreviewCover(Option<Vec<u8>>, i64, u64),
    MetadataCoverArt(Option<Vec<u8>>, i64, u64),
    ArtistCoverArt(Option<Vec<u8>>, String, u64),
    SpotifyPreviewCover(Option<Vec<u8>>, String, u64),
    CoverPicker(Option<Picker>),
    Lyrics(Option<LrcData>, u64),
    /// Authorize URL produced by the daemon's OAuth flow. Kept separate from
    /// `Notification` so the SpotifyLink picker can render it inline.
    SpotifyOauthUrl(String),
    /// A hard failure of the OAuth flow (daemon could not even start it).
    SpotifyOauthError(String),
    LibraryTracks(Vec<TrackInfo>),
    PlaylistTracks(Vec<TrackInfo>),
    Playlists(Vec<Playlist>),
    /// A new playlist was created; carry its id + name so the TUI can open the
    /// track multi-select picker to populate it.
    PlaylistCreated(i64, String),
    Queue(Vec<TrackInfo>, usize),
    YtResults(String, Vec<YTSearchResult>),
    /// Live yt-dlp download state, mirrored from the daemon's
    /// `YtDownloadProgress` poll so the footer can show a live progress.
    YtDownloadProgress {
        id: u64,
        url: String,
        title: String,
        progress: f64,
        status: String,
        file_path: Option<String>,
        downloaded_bytes: Option<u64>,
        total_bytes: Option<u64>,
        rate_bytes_per_sec: Option<f64>,
        eta_secs: Option<u64>,
    },
    Notification(String, String, NotificationKind, NotifType),
    Error(String),
    HealthReport(HealthReport),
    SpotifyStatus(SpotifyStatus),
    SpotifyPlaylists(Vec<SpotifyPlaylist>),
    SpotifyTracks(Vec<SpotifyTrack>),
    SpotifySearchWebResults(u64, Vec<SpotifyTrack>),
    ReactivePalette(Option<ReactivePalette>),
    SubsonicStatus(Option<SubsonicStatus>),
    SubsonicSearch(SubsonicSearchResults),
    SubsonicAlbums(Vec<SubsonicAlbum>),
    SubsonicAlbumTracks(Vec<SubsonicTrack>),
    SubsonicCover(Option<String>),
    PodcastStatus(Option<PodcastStatus>),
    PodcastFeeds(Vec<PodcastFeed>),
    PodcastEpisodes(Vec<PodcastEpisode>),
    RadioSearch(Vec<RadioStation>),
    RadioTop(Vec<RadioStation>),
    RadioTags(Vec<RadioTag>),
    RadioCountries(Vec<RadioCountry>),
    RadioBrowseStations(Vec<RadioStation>),
    /// Last.fm link status refreshed after a setup action completes.
    LastfmStatus(Option<LastfmStatus>),
    /// Authorization URL produced by the daemon's Last.fm auth flow.
    LastfmAuthUrl(String),
    /// Hard failure of the Last.fm setup flow.
    LastfmAuthError(String),
}

/// Send a background-task error into the TUI event stream as an Error
/// (surfaced in the notification history).
fn self_err(ipc_tx: &mpsc::UnboundedSender<IpcResult>, msg: String) {
    let _ = ipc_tx.send(IpcResult::Error(msg));
}

/// Best-effort browser open for the OAuth authorize URL. Tries common
/// openers until one succeeds; when all fail, the URL is sent back through the
/// TUI as a notification so the user can open it manually (and copy it from
/// the notification history).
fn try_open_browser(url: &str, ipc_tx: &mpsc::UnboundedSender<IpcResult>) {
    let url = url.to_string();
    let ipc_tx = ipc_tx.clone();
    tokio::spawn(async move {
        // Prefer the OS default browser opener, which is cross-platform. Guard
        // it with a timeout: a wedged `xdg-open`-style launcher must not leave
        // the flow looking dead.
        let opened = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::task::spawn_blocking({
                let url = url.clone();
                move || webbrowser::open(&url)
            })
            .await
        })
        .await;
        if let Ok(Ok(Ok(_))) = opened {
            return;
        }
        // Fallback to common launchers when the `webbrowser` crate can't
        // resolve one (e.g. minimal containers / WSL).
        for prog in ["xdg-open", "open", "start"] {
            match tokio::process::Command::new(prog)
                .arg(&url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await
            {
                Ok(st) if st.success() => return,
                _ => continue,
            }
        }
        // All openers failed: surface the URL in the TUI so it can be copied.
        let _ = ipc_tx.send(IpcResult::Notification(
            "Spotify".to_string(),
            format!("Open this URL in your browser to authorize gtm:\n{url}"),
            NotificationKind::Info,
            NotifType::Spotify,
        ));
        let _ = ipc_tx.send(IpcResult::SpotifyOauthError(
            "Could not open a browser automatically — copy the URL from the notification".into(),
        ));
    });
}

/// Validate a typed Spotify client id before starting the PKCE flow, and
/// remind the user of the redirect-URI requirement: when the URI is missing
/// from the app dashboard the flow fails silently inside the browser (see
/// `docs/spec/spotify-linking.md`). The empty-input fallback (librespot's
/// public desktop id) always passes this check.
fn spotify_client_id_error(client_id: &str, port: u16) -> Option<String> {
    if client_id.len() != 32 || !client_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!(
            "This doesn't look like a valid Spotify Client ID (32 hex chars).\n\
             Also make sure your app lists http://127.0.0.1:{port}/login as a\n\
             Redirect URI (127.0.0.1, not localhost) or the link fails silently."
        ));
    }
    None
}

fn spawn_sync_and_wait(
    c: DaemonClient,
    kind: SyncKind,
    label: &'static str,
    ipc_tx: mpsc::UnboundedSender<IpcResult>,
) {
    tokio::spawn(async move {
        let kick = match kind {
            SyncKind::Covers => c.library().sync_covers().await,
            SyncKind::Lyrics => c.library().sync_lyrics().await,
            SyncKind::Metadata => c.library().sync_metadata(None).await,
        };
        if let Err(e) = kick {
            let _ = ipc_tx.send(IpcResult::Error(format!("{label} sync failed: {e}")));
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1800);
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            match c.library().sync_status().await {
                Ok(st) if !st.running => {
                    let msg = format!("{label} synced: {}/{} tracks", st.synced, st.total);
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Library".to_string(),
                        msg,
                        NotificationKind::Info,
                        NotifType::Library,
                    ));
                    if let Ok(DaemonRes::Tracks { tracks, .. }) =
                        c.library().get_tracks(None, None).await
                    {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                    break;
                }
                Ok(_) if std::time::Instant::now() >= deadline => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("{label} sync timed out")));
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    break;
                }
            }
        }
    });
}

pub enum TuiCommand {
    Play(String),
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek(f64),
    SetVolume(u8),
    SetSpeed(f32),
    SetLowPower(bool),
    ToggleShuffle,
    CycleRepeat(RepeatMode),
    ToggleMute,
    Crossfade(bool, u8),
    QueueAdd(String),
    QueueMove(u64, u64),
    QueueClear,
    YtSearch(String),
    YtDownload {
        url: String,
        title: Option<String>,
        artist: Option<String>,
    },
    YtResolve(String),
    SetEqPreset(EqPreset),
    Search(String),
    AddFavourite(i64),
    RemoveFavourite(i64),
    Refresh,
    RefreshLibrary,
    RefreshYt,
    RemoveTrack(i64),
    RemoveFromPlaylist(i64, i64),
    FetchLyrics,
    SetSleepTimer(u32),
    CancelSleepTimer,
    CheckHealth,
}

impl App {
    pub fn surface_bg(&self) -> ratatui::style::Color {
        if self.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            self.theme.bg
        }
    }

    pub fn pane_surface_bg(&self) -> ratatui::style::Color {
        if self.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            self.theme.pane_bg
        }
    }

    pub fn float_bg(&self) -> ratatui::style::Color {
        if self.transparent_bg {
            blend_colors(self.theme.elevated_bg, self.theme.bg, 0.5)
        } else {
            self.theme.elevated_bg
        }
    }

    pub fn chrome_bg(&self) -> ratatui::style::Color {
        if self.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            self.theme.border
        }
    }

    fn next_cover_gen(&mut self) -> u64 {
        let g = self.next_cover_gen;
        self.next_cover_gen = self.next_cover_gen.wrapping_add(1).max(1);
        g
    }

    fn next_lyrics_gen(&mut self) -> u64 {
        let g = self.lyrics.next_gen;
        self.lyrics.next_gen = self.lyrics.next_gen.wrapping_add(1).max(1);
        g
    }

    fn clear_search_previews(&mut self) {
        self.picker_preview_cover = None;
        self.picker_preview_stateful = None;
        self.last_picker_preview_fetch_id = None;
        self.last_picker_preview_fetch_gen = None;
        self.artist_cover = None;
        self.artist_cover_stateful = None;
        self.last_artist_cover_fetch = None;
        self.last_artist_cover_fetch_gen = None;
    }

    fn clear_preview(&mut self) {
        self.queue.preview_cover = None;
        self.queue.preview_cover_stateful = None;
        self.queue.last_preview_cover_fetch_id = None;
        self.queue.last_preview_cover_fetch_gen = None;
    }

    fn clear_popup_cover(&mut self) {
        self.track_popup_track_id = None;
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        self.last_popup_cover_fetch_id = None;
        self.last_popup_cover_fetch_gen = None;
    }

    pub async fn new(
        socket_path: &Path,
        setup_service: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = DaemonClient::connect(socket_path).await?;
        let state = DaemonState::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (high_pri_cmd_tx, high_pri_cmd_rx) = mpsc::unbounded_channel();
        let (ipc_tx, ipc_rx) = mpsc::unbounded_channel();
        let prefs = tokio::task::spawn_blocking(load_prefs)
            .await
            .unwrap_or_else(|_| Prefs::default());
        let keybindings = build_keybindings(&prefs.keybindings);
        let initial_cursor = state.queue_cursor;

        // Build the merged theme + footer preset tables (built-ins overridden
        // by user-supplied files under ~/.config/gtm/). Resolve the persisted
        // prefs by name so adding/removing a built-in never shifts the saved
        // theme off its slot.
        let themes = merged_themes();
        let theme_index = resolve_theme_index(&themes, &prefs.theme_name, &prefs.theme_mode);
        let theme = if themes.is_empty() {
            chadrula()
        } else {
            themes[theme_index].theme
        };
        // Similar to themes: resolve the footer preset by name so adding or
        // removing built-in presets never shifts a saved slot.
        let footer_presets = merged_presets();
        let footer_preset = footer_presets
            .iter()
            .position(|p| p.name == prefs.footer_preset_name)
            .unwrap_or(0);
        let mut app = Self {
            theme,
            themes,
            client,
            state,
            display_position: 0.0,
            last_display_position: 0.0,
            raw_position: 0.0,
            seek_pending: None,
            seek_cmd_accum: None,
            last_seek_press: None,
            progress_smoother: ProgressSmoother::new(),
            last_frame: std::time::Instant::now(),
            frame_count: 0,
            scanner_pos: 0,
            scanner_dir: 1,
            scanner_hold: 0,
            cursor_blink: true,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            scroll_offset: [0; LIBRARY_CATEGORIES.len()],
            library_category: 0,
            library_pane_focus: false,
            settings_category: 0,
            settings_pane_focus: false,
            settings_option: 0,
            tracks_cache: Vec::new(),
            queue: QueueView {
                cache: Vec::new(),
                cursor: 0,
                move_index: None,
                move_target: 0,
                preview_cover: None,
                preview_cover_stateful: None,
                last_preview_cover_fetch_id: None,
                last_preview_cover_fetch_gen: None,
                preview_cover_fail_until: None,
            },
            browse_detail: None,
            yt_results_cache: Vec::new(),
            downloads: std::collections::HashMap::new(),
            playlist_cache: Vec::new(),
            playlist_tracks_cache: Vec::new(),
            spotify: SpotifyView {
                status: None,
                playlists: Vec::new(),
                playlist_tracks_cache: Vec::new(),
                search_results: Vec::new(),
                link_input: String::new(),
                token_input: String::new(),
                oauth_pending: false,
                oauth_url: None,
                oauth_error: None,
                oauth_port: "8990".to_string(),
                link_field: 0,
                search_debounce: None,
                web_seq: 0,
                preview_cover: None,
                preview_cover_stateful: None,
                last_preview_fetch: None,
                last_preview_fetch_gen: None,
            },
            subsonic: SubsonicView::default(),
            podcast: PodcastView::default(),
            radio: RadioView::default(),
            cookie_file: None,
            notifications: Vec::new(),
            notification_history: Vec::new(),
            footer_notification: None,
            notification_modes: default_notification_modes()
                .into_iter()
                .map(|(k, v)| (NotifType::from_str_lossy(&k), NotifMode::from_str_lossy(&v)))
                .collect(),
            cover_provider: prefs.cover_provider.clone(),
            auto_fetch_lyrics: prefs.auto_fetch_lyrics,
            icon_style: prefs.icon_style.clone(),
            crossfade_duration: 6,
            pending_delete: None,
            pending_prompt: None,
            yt_search_loading: false,
            yt_search_debounce: None,
            yt_search_poll_deadline: None,
            pickers: PickerManager::new(),
            sleep_timer: SleepTimerState {
                remaining: None,
                minutes: 30,
                input_mode: false,
                input_buf: String::new(),
            },
            np_cover: NowPlayingCoverState {
                image: None,
                track_id: None,
                track_path: None,
                picker: None,
                stateful: None,
                pending_gen: None,
            },
            terminal_cols: 80,
            terminal_rows: 24,
            cmd_rx,
            cmd_tx,
            high_pri_cmd_rx,
            high_pri_cmd_tx,
            ipc_rx,
            ipc_tx,
            keybindings,
            prefs_keybindings: prefs.keybindings.clone(),
            theme_index,
            list_scroll: 0,
            viewport_items: 20,
            transparent_bg: prefs.transparent_bg,
            transparent_pickers: prefs.transparent_pickers,
            reactive_theme: prefs.reactive_theme,
            reactive_theme_intensity: prefs.reactive_theme_intensity,
            reactive_palette: None,
            last_action_name: None,
            footer_title_scroll: 0,
            footer_time_format: if prefs.time_format.is_empty() {
                default_time_format()
            } else {
                prefs.time_format.clone()
            },
            theme_mode: if prefs.theme_mode.is_empty() {
                default_theme_mode()
            } else {
                prefs.theme_mode.clone()
            },
            track_sort: prefs.track_sort,
            is_ready: false,
            last_queue_cursor: initial_cursor,
            manual_track_advance: false,
            auto_track_advance: false,
            last_track_path_display: None,
            prev_track_id: None,
            prev_status: PlaybackStatus::Stopped,
            prev_volume: 100,
            prev_cover_id: None,
            cover_art_dirty: false,
            footer_cache: FooterCache::default(),
            footer_presets,
            footer_preset,
            last_event_time: std::time::Instant::now(),
            multiselect_mode: false,
            progress_style: prefs.progress_style,
            visualizer: {
                let mut v = AudioVisualizer::new();
                v.preset = prefs.visualizer_preset;
                v
            },
            selected_indices: std::collections::HashSet::new(),
            pending_motion: None,
            pending_playlist_track_ids: Vec::new(),
            pending_playlist_id: None,
            selected_playlist_track_ids: std::collections::HashSet::new(),
            playlist_creating: false,
            metadata: MetadataEditState {
                edit_track_id: None,
                fields: Default::default(),
                field_idx: 0,
                cover: None,
                cover_stateful: None,
                cover_dirty: false,
                cover_fetch_gen: None,
            },
            pending_quit: false,
            mouse_map: MouseMap::default(),
            np_title_scroll: 0,
            track_anim_trigger: false,
            anim_fx: EffectManager::default(),
            track_popup_visible: false,
            track_popup_track_id: None,
            track_popup_cover: None,
            popup_cover_stateful: None,
            last_popup_cover_fetch_id: None,
            last_popup_cover_fetch_gen: None,
            picker_preview_cover: None,
            picker_preview_stateful: None,
            last_picker_preview_fetch_id: None,
            last_picker_preview_fetch_gen: None,
            artist_cover: None,
            artist_cover_stateful: None,
            last_artist_cover_fetch: None,
            last_artist_cover_fetch_gen: None,
            upnext: None,
            next_cover_gen: 1,
            lyrics: LyricsView {
                current: None,
                scroll: 0,
                fetching: false,
                pending_gen: None,
                next_gen: 1,
                show: false,
                pane_focus: false,
                manual_scroll: false,
            },
            show_health_panel: false,
            show_health_on_report: false,
            health_report: None,
            hide_help_bar: true,
            hide_footer: false,
            pending_suspend: false,
            setup: SetupView::default(),
            last_config_mtime: std::fs::metadata(prefs_path())
                .ok()
                .and_then(|m| m.modified().ok()),
        };
        app.open_setup_picker(setup_service.as_deref());
        Ok(app)
    }

    /// Open the `gtm setup` walkthrough, either the service chooser or the
    /// matching setup picker directly. Called from `gtm setup SERVICE`.
    pub fn open_setup_picker(&mut self, service: Option<&str>) {
        match service.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("spotify") => {
                self.setup.selection = 0;
                self.pickers.open(PickerId::SpotifyLink);
                self.open_spotify_link();
            }
            Some("lastfm" | "last.fm") => {
                self.setup.selection = 1;
                self.pickers.open(PickerId::LastfmAuth);
                self.on_picker_opened(PickerId::LastfmAuth);
            }
            Some("subsonic") | Some("navidrome") => {
                self.setup.selection = 2;
                self.pickers.open(PickerId::SubsonicSetup);
                self.on_picker_opened(PickerId::SubsonicSetup);
            }
            _ => {
                self.pickers.open(PickerId::Setup);
                self.on_picker_opened(PickerId::Setup);
            }
        }
    }

    /// Kick off the Spotify OAuth browser flow, opening the picker and
    /// requesting the authorize URL from the daemon.
    pub fn open_spotify_link(&mut self) {
        let client_id = self.spotify.link_input.trim().to_string();
        let client_id = if client_id.is_empty() {
            LIBRESPOT_CLIENT_ID.to_string()
        } else {
            client_id
        };
        let port = self
            .spotify
            .oauth_port
            .trim()
            .parse::<u16>()
            .unwrap_or(8990);
        self.start_spotify_oauth(client_id, port);
    }

    /// Start the Spotify OAuth PKCE flow for `client_id` on `port` and watch it
    /// in the background. Validates the client id first (the empty-input
    /// fallback id always passes); an invalid id or a missing redirect-URI
    /// registration is reported inline instead of silently dying in the
    /// browser.
    fn start_spotify_oauth(&mut self, client_id: String, port: u16) {
        if let Some(err) = spotify_client_id_error(&client_id, port) {
            self.spotify.oauth_error = Some(err);
            self.spotify.oauth_pending = false;
            self.spotify.link_input.clear();
            return;
        }
        set_secret(SPOTIFY_CLIENT_ID_KEY, &client_id);
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        self.spotify.link_input.clear();
        self.spotify.oauth_pending = true;
        self.spotify.oauth_url = None;
        self.spotify.oauth_error = None;
        tokio::spawn(async move {
            match c.spotify().oauth_start(&client_id, port).await {
                Ok(url) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyOauthUrl(url.clone()));
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Spotify".to_string(),
                        "Authorize gtm in your browser, then playlists sync automatically…"
                            .to_string(),
                        NotificationKind::Info,
                        NotifType::Spotify,
                    ));
                    try_open_browser(&url, &ipc_tx);
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyOauthError(format!(
                        "Spotify link failed: {e}"
                    )));
                }
            }
        });
    }

    /// Check if config.toml was modified since last load; if so, re-parse
    /// and apply hot-reloadable settings (theme, transparent_bg,
    /// progress_style, visualizer_preset, keybindings).
    fn check_config_reload(&mut self) {
        let path = prefs_path();
        let mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.last_config_mtime == Some(mtime) {
            return;
        }
        self.last_config_mtime = Some(mtime);
        let prefs = load_prefs();

        // Theme
        let theme_index = resolve_theme_index(&self.themes, &prefs.theme_name, &prefs.theme_mode);
        self.theme_index = theme_index;
        self.theme_mode = if prefs.theme_mode.is_empty() {
            default_theme_mode()
        } else {
            prefs.theme_mode.clone()
        };
        self.apply_reactive();

        // Transparent bg
        self.transparent_bg = prefs.transparent_bg;
        self.transparent_pickers = prefs.transparent_pickers;
        self.reactive_theme = prefs.reactive_theme;
        self.reactive_theme_intensity = prefs.reactive_theme_intensity;
        // Reactive palette may be retained from before a reload; re-derive
        // the theme so a changed intensity/wash applies immediately.
        self.apply_reactive();

        self.footer_cache.suppress_refresh = true;

        // Footer preset
        let footer_preset = self
            .footer_presets
            .iter()
            .position(|p| p.name == prefs.footer_preset_name)
            .unwrap_or(0)
            .min(self.footer_presets.len().saturating_sub(1));
        self.footer_preset = footer_preset;

        // Progress style
        self.progress_style = prefs.progress_style;

        // Visualizer preset
        self.visualizer.preset = prefs.visualizer_preset;

        // Footer time format
        self.footer_time_format = if prefs.time_format.is_empty() {
            default_time_format()
        } else {
            prefs.time_format.clone()
        };

        // Keybindings
        self.prefs_keybindings = prefs.keybindings.clone();
        self.keybindings = build_keybindings(&prefs.keybindings);

        // Per-type notification modes (defaults used for types not present in
        // an older config file).
        self.notification_modes.clear();
        for (k, v) in &prefs.notification_modes {
            let t = NotifType::from_str_lossy(k);
            let m = NotifMode::from_str_lossy(v);
            self.notification_modes.insert(t, m);
        }

        // Hide footer
        self.hide_footer = prefs.hide_footer;
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    pub fn send_high(&self, cmd: TuiCommand) {
        let _ = self.high_pri_cmd_tx.send(cmd);
    }

    /// Snapshot of the user-facing preferences resolved from current indices.
    /// Stored by name (not index) so adding/removing built-in themes or
    /// footer presets doesn't shift what was saved.
    fn current_prefs(&self) -> Prefs {
        Prefs {
            theme_name: self
                .themes
                .get(self.theme_index)
                .map(|t| t.name.to_string())
                .unwrap_or_else(default_theme_name),
            transparent_bg: self.transparent_bg,
            transparent_pickers: self.transparent_pickers,
            reactive_theme: self.reactive_theme,
            reactive_theme_intensity: self.reactive_theme_intensity,
            footer_preset_name: self
                .footer_presets
                .get(self.footer_preset)
                .map(|p| p.name.to_string())
                .unwrap_or_else(default_footer_preset_name),
            progress_style: self.progress_style,
            visualizer_preset: self.visualizer.preset,
            time_format: self.footer_time_format.clone(),
            theme_mode: self.theme_mode.clone(),
            track_sort: self.track_sort,
            keybindings: self.prefs_keybindings.clone(),
            notification_modes: self
                .notification_modes
                .iter()
                .map(|(t, m)| (t.as_str().to_string(), m.as_str().to_string()))
                .collect(),
            cover_provider: self.cover_provider.clone(),
            auto_fetch_lyrics: self.auto_fetch_lyrics,
            icon_style: self.icon_style.clone(),
            hide_footer: self.hide_footer,
        }
    }

    /// Apply a theme picker selection by index, persist by name, and refresh
    /// the live `theme` field.
    fn apply_theme_index(&mut self, idx: usize) {
        let idx = idx.min(self.themes.len().saturating_sub(1));
        self.theme_index = idx;
        self.apply_reactive();
        // A manual selection opts out of OS-theme auto-detection so the OS
        // preference can't fight the choice on restart.
        self.theme_mode = "manual".to_string();
        save_prefs(&self.current_prefs());
    }

    /// Cycle the theme-following mode (auto → dark → light → manual → auto),
    /// immediately re-resolve the matching theme and apply it, then persist so
    /// the choice survives restarts. `manual` keeps the current pick; `auto`
    /// re-enables OS light/dark detection.
    fn cycle_theme_mode(&mut self) {
        let next_mode = match self.theme_mode.trim().to_ascii_lowercase().as_str() {
            "auto" => "dark",
            "dark" => "light",
            "light" => "manual",
            "manual" => "auto",
            _ => "auto",
        }
        .to_string();
        self.theme_mode = next_mode;
        let saved_name = self
            .themes
            .get(self.theme_index)
            .map(|t| t.name.to_string())
            .unwrap_or_else(default_theme_name);
        let idx = resolve_theme_index(&self.themes, &saved_name, &self.theme_mode);
        self.theme_index = idx;
        self.apply_reactive();
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "Theme",
            format!("Theme mode: {}", theme_mode_label(&self.theme_mode)),
            NotificationKind::Info,
            true,
            NotifType::Prefs,
        );
    }

    /// Cycle the cover-art provider (auto → musicbrainz → deezer → spotify →
    /// auto), push the live value to the daemon and persist it for restarts.
    fn cycle_cover_provider(&mut self) {
        let next = match self.cover_provider.trim().to_ascii_lowercase().as_str() {
            "auto" => "musicbrainz",
            "musicbrainz" | "mb" => "deezer",
            "deezer" => "spotify",
            _ => "auto",
        }
        .to_string();
        self.cover_provider = next.clone();
        let label = cover_provider_label(&next);
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.set_cover_provider(&next).await;
        });
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "System",
            format!("Cover source: {}", label),
            NotificationKind::Info,
            false,
            NotifType::Prefs,
        );
    }

    /// Apply a footer-preset picker selection by index and persist by name.
    fn apply_footer_preset_index(&mut self, idx: usize) {
        let idx = idx.min(self.footer_presets.len().saturating_sub(1));
        self.footer_preset = idx;
        save_prefs(&self.current_prefs());
    }

    /// Recompute the live theme: the base preset, re-tinted by the reactive
    /// cover palette when reactive theming is enabled.
    fn apply_reactive(&mut self) {
        let Some(entry) = self.themes.get(self.theme_index) else {
            return;
        };
        let light = entry.light;
        let base = entry.theme;
        self.theme = match (self.reactive_theme, self.reactive_palette) {
            (true, Some(pal)) => derive_theme(&base, &pal, light, self.reactive_theme_intensity),
            _ => base,
        };
    }

    /// Cycle the reactive background wash strength between presets, persist,
    /// and re-apply the reactive theme so the change is visible immediately.
    fn cycle_reactive_intensity(&mut self) {
        const STEPS: [f32; 5] = [0.15, 0.25, 0.34, 0.45, 0.6];
        let cur = STEPS
            .iter()
            .position(|v| (v - self.reactive_theme_intensity).abs() < 1e-3);
        let next = match cur {
            Some(i) => (i + 1) % STEPS.len(),
            // Exact-match on a custom value: advance to the next preset, or
            // wrap back to the strongest one.
            None => STEPS
                .iter()
                .position(|v| *v > self.reactive_theme_intensity)
                .unwrap_or(0),
        };
        self.reactive_theme_intensity = STEPS[next];
        self.apply_reactive();
        save_prefs(&self.current_prefs());
        self.notify_titled(
            "Reactive Theme",
            format!("Intensity: {:.0}%", self.reactive_theme_intensity * 100.0),
            NotificationKind::Info,
            true,
            NotifType::Prefs,
        );
    }

    /// Kick off palette extraction for freshly received cover art.  Runs on
    /// a blocking thread; the result comes back through
    /// [`IpcResult::ReactivePalette`].
    fn request_reactive_palette(
        &self,
        cover_bytes: &[u8],
        ipc_tx: mpsc::UnboundedSender<IpcResult>,
    ) {
        let bytes = cover_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let pal = extract_palette(&bytes);
            let _ = ipc_tx.send(IpcResult::ReactivePalette(pal));
        });
    }

    /// Cycle to the next theme in the list, persist, and refresh.
    fn toggle_theme(&mut self) {
        let next = (self.theme_index + 1) % self.themes.len();
        self.apply_theme_index(next);
        // A manual toggle opts out of OS-theme auto-detection until the user
        // returns to "auto" mode, so the OS preference can't fight the choice.
        self.theme_mode = "manual".to_string();
        let name = &self.themes[next].name;
        let light = if self.themes[next].light {
            " (light)"
        } else {
            ""
        };
        self.notify_titled(
            "Theme",
            format!("Theme: {}{}", name, light),
            NotificationKind::Info,
            true,
            NotifType::Prefs,
        );
    }

    /// Cycle the library track list sort order and persist the selection.
    fn cycle_track_sort(&mut self) {
        self.track_sort = self.track_sort.next();
        self.notify_titled(
            "Sort",
            format!("Sorting by: {}", self.track_sort.label()),
            NotificationKind::Info,
            true,
            NotifType::Prefs,
        );
        save_prefs(&self.current_prefs());
    }

    /// Accumulate a seek delta from a repeated (held) key press. Updates the
    /// local position estimate immediately for smooth feedback and only
    /// defers the authoritative daemon seek to `ensure_seek_flush`, so a
    /// long-press never floods the daemon with full re-decodes.
    fn accumulate_seek(&mut self, delta: f64) {
        if self.state.current_track.is_none() {
            return;
        }
        let raw = self.seek_cmd_accum.unwrap_or(self.raw_position) + delta;
        self.seek_cmd_accum = Some(raw);
        self.last_seek_press = Some(std::time::Instant::now());
        let clamped = raw.clamp(0.0, self.state.duration.max(0.0));
        // Immediate local feedback; the daemon catches up on flush.
        self.raw_position = clamped;
        self.display_position = clamped;
        self.seek_pending = Some(std::time::Instant::now());
    }

    /// Send the coalesced seek to the daemon once the user stops hammering the
    /// seek key (or a full command seek was requested). Called every frame.
    fn ensure_seek_flush(&mut self) {
        let Some(accum) = self.seek_cmd_accum else {
            return;
        };
        // Flush once no repeat press has arrived for a short window.
        let idle = self
            .last_seek_press
            .map(|t| t.elapsed())
            .unwrap_or(std::time::Duration::ZERO)
            >= std::time::Duration::from_millis(160);
        if idle {
            let pos = accum.clamp(0.0, self.state.duration.max(0.0));
            self.send_high(TuiCommand::Seek(pos));
            self.seek_cmd_accum = None;
            self.last_seek_press = None;
        }
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // All initial IPC is done in background tasks so the TUI renders
        // immediately with an empty state. Results arrive via ipc_rx and
        // are applied on the next loop iteration.
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(state) = c.get_status().await {
                    let _ = ipc_tx.send(IpcResult::RefreshDone(Box::new(state), None, None));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::QueueState {
                    queue: tracks,
                    cursor,
                    ..
                }) = c.queue().list().await
                {
                    let _ = ipc_tx.send(IpcResult::Queue(tracks, cursor as usize));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::Tracks { tracks, .. }) =
                    c.library().get_tracks(None, None).await
                    && !tracks.is_empty()
                {
                    let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(status) = c.spotify().status().await {
                    let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                }
                if let Ok(playlists) = c.spotify().playlists().await {
                    let _ = ipc_tx.send(IpcResult::SpotifyPlaylists(playlists));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(report) = c.check_health().await {
                    let _ = ipc_tx.send(IpcResult::HealthReport(report));
                }
            });
        }

        // Initialize cover image picker in background (blocking terminal query).
        {
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                let picker = tokio::task::spawn_blocking(|| Picker::from_query_stdio().ok())
                    .await
                    .unwrap_or(None);
                let _ = ipc_tx.send(IpcResult::CoverPicker(picker));
            });
        }

        self.is_ready = true;

        // Seed the track-info popup so the first row's details are visible
        // immediately when the middle pane starts focused.
        self.update_track_popup();

        // Animate the initial frame so the library list and Now Playing pane
        // evolve into view on startup.
        self.track_anim_trigger = true;

        // Render the initial frame immediately, before the main loop, so
        // the user never sees a blank alternate screen on startup.
        let _ = terminal.draw(|f| ui::render(f, &mut self));

        let cmd_tx = self.cmd_tx();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let _ = cmd_tx.try_send(TuiCommand::Refresh);
            }
        });

        'outer: loop {
            // Input-first: process any already-buffered terminal events before
            // state work and rendering so key actions are applied immediately.
            while event::poll(Duration::ZERO).unwrap_or(false) {
                match event::read() {
                    Ok(ev) => {
                        if !self.handle_terminal_event(ev).await {
                            break 'outer;
                        }
                    }
                    Err(_) => break,
                }
            }
            let mut events_received = false;
            let mut had_track_change = false;
            let mut had_sleep_expired = false;
            let mut had_sync_done = false;
            let mut had_spotify_change = false;
            for ev in self.client.drain().await {
                if let DaemonEvent::PlaybackStarted { .. } = &ev {
                    // The crossfade has begun: drop the Up Next countdown.
                    self.upnext = None;
                    // Was this change an automatic advance (not a manual
                    // Next/Prev)? Capture that before the flag is reset so the
                    // dust animation is only shown on genuine auto-advances.
                    self.auto_track_advance = !self.manual_track_advance;
                    // Any PlaybackStarted consumes the manual-advance flag.
                    self.manual_track_advance = false;
                }
                if matches!(ev, DaemonEvent::PlaybackStarted { .. }) {
                    had_track_change = true;
                }
                if matches!(ev, DaemonEvent::TrackEnded) {
                    self.upnext = None;
                }
                if matches!(ev, DaemonEvent::SleepTimerExpired) {
                    had_sleep_expired = true;
                }
                if let DaemonEvent::CrossfadeCountdown { track } = &ev
                    // Only surface the crossfade/Up Next card on a genuine
                    // auto-advance; a manual Next/Prev shouldn't announce it.
                    && !self.manual_track_advance
                {
                    self.start_upnext(track.clone());
                }
                // The daemon finished an OAuth link flow: pull the fresh
                // status + playlists so they appear without a restart.
                if matches!(ev, DaemonEvent::SpotifyStatusChanged) {
                    had_spotify_change = true;
                }
                // After a background metadata sync finishes, re-pull the
                // library so scrubbed tags / fetched covers show up live.
                if let DaemonEvent::Custom { name, data } = &ev
                    && name == "sync_done"
                    && data.get("kind").is_some_and(|k| k == "metadata")
                {
                    had_sync_done = true;
                }
                self.state.apply_event(&ev);
                events_received = true;
            }
            // After track change, check if new track has cover art.
            // If not, clear reactive palette so theme reverts to base.
            if had_track_change && self.reactive_theme {
                let has_cover = self
                    .state
                    .current_track
                    .as_ref()
                    .map(|t| t.cover_path.is_some())
                    .unwrap_or(false);
                if !has_cover {
                    self.reactive_palette = None;
                    self.apply_reactive();
                }
            }
            // If the countdown elapsed without a PlaybackStarted (e.g. the
            // track ended before the crossfade could fire), drop the card.
            if let Some(u) = self.upnext.as_ref()
                && u.started_at.elapsed().as_secs_f64() >= u.total_secs
            {
                self.upnext = None;
            }
            if events_received {
                self.last_event_time = std::time::Instant::now();
            }
            if had_sleep_expired {
                self.sleep_timer.remaining = None;
                self.notify_titled(
                    "Sleep Timer",
                    "Sleep timer expired: shutting down",
                    NotificationKind::Info,
                    false,
                    NotifType::Playback,
                );
                // Draw a final frame so the user sees the shutdown message
                // before the terminal restores.
                let _ = terminal.draw(|f| ui::render(f, &mut self));
                tokio::time::sleep(Duration::from_millis(1200)).await;
                // Await the daemon's reply so the quit request is actually
                // delivered before the TUI exits. The daemon replies Ok then
                // shuts down ~200ms later.
                let c = self.client.clone();
                let _ = tokio::time::timeout(Duration::from_millis(1500), c.quit()).await;
                break;
            }
            // Sync sleep_timer_remaining from daemon state
            if let Some(secs) = self.state.sleep_timer {
                self.sleep_timer.remaining = Some(secs as u64);
            } else if self.sleep_timer.remaining.is_some() && self.state.sleep_timer.is_none() {
                self.sleep_timer.remaining = None;
            }
            // Re-seed clock from state after track change events so the
            // local position estimate stays in sync with the daemon.
            if had_track_change {
                self.client.seed_clock_from_state(&self.state).await;
            }
            if had_sync_done
                && let Ok(DaemonRes::Tracks { tracks, .. }) =
                    self.client.library().get_tracks(None, None).await
            {
                self.tracks_cache = tracks;
            }

            if had_spotify_change {
                if let Ok(status) = self.client.spotify().status().await {
                    let was_linked = self.spotify.status.as_ref().is_some_and(|s| s.linked);
                    if status.linked && !was_linked {
                        let user = status.user.clone().unwrap_or_else(|| "account".into());
                        self.notify_titled(
                            "Spotify",
                            format!("Linked as {user} — playlists synced"),
                            NotificationKind::Success,
                            false,
                            NotifType::Spotify,
                        );
                        // The OAuth browser flow finished: dismiss the waiting
                        // picker if it's still open and navigate to Spotify.
                        self.spotify.oauth_pending = false;
                        self.spotify.oauth_url = None;
                        self.spotify.oauth_error = None;
                        if self
                            .pickers
                            .top()
                            .is_some_and(|o| o.id == PickerId::SpotifyLink)
                        {
                            self.close_top_picker_with_cleanup();
                        }
                        self.reset_library_view(5, None);
                        self.library_pane_focus = true;
                    } else if self.spotify.oauth_pending && !status.linked {
                        // The OAuth browser flow failed (e.g. no network): stop
                        // waiting, dismiss the picker and report the failure.
                        let msg = status
                            .error
                            .clone()
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| "Spotify link failed".to_string());
                        self.spotify.oauth_pending = false;
                        // Keep the picker open and render the error inline: a
                        // floating toast is suppressed while a picker is open,
                        // so dismissing here would drop the only feedback.
                        self.spotify.oauth_error = Some(msg);
                        // The authorize URL (set when the flow started) stays
                        // visible so the user can retry from the browser side.
                        self.notify_titled(
                            "Spotify",
                            "Spotify link failed",
                            NotificationKind::Error,
                            false,
                            NotifType::Spotify,
                        );
                    }
                    self.spotify.status = Some(status);
                }
                if let Ok(playlists) = self.client.spotify().playlists().await {
                    self.spotify.playlists = playlists;
                }
            }

            // Force a state refresh if no events received for 8s to prevent
            // stale state from broadcast lag. Works in all playback states,
            // not just Playing, to catch lag when paused/stopped too.
            if self.last_event_time.elapsed() > Duration::from_secs(8) && self.client.is_connected()
            {
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = c.get_status().await {
                        let _ = ipc_tx.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                });
                self.last_event_time = std::time::Instant::now();
            }

            self.last_queue_cursor = self.state.queue_cursor;

            // Track-change detection keyed on path.  Queued/foreign tracks
            // share `id == 0`, so id-based detection misses auto-advance
            // between them (stale elapsed, cover art and lyrics).
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            let current_path = self.state.current_track.as_ref().map(|t| t.path.clone());
            let track_changed = current_path.as_deref() != self.last_track_path_display.as_deref();
            if track_changed {
                self.last_track_path_display = current_path;
                let raw = self.client.estimated_position().await;
                self.display_position = raw;
                self.last_display_position = raw;
                self.raw_position = raw;
                let d = if self.state.duration > 0.0 {
                    self.state.duration
                } else {
                    self.state
                        .current_track
                        .as_ref()
                        .map(|t| t.duration)
                        .unwrap_or(0.0)
                };
                self.progress_smoother
                    .reset(if d > 0.0 { raw / d } else { 0.0 });
                self.track_anim_trigger = true;
                // Re-enable auto-sync for the new track; a manual lyric scroll
                // on a previous track must not stick across track changes.
                self.lyrics.manual_scroll = false;
            }

            // Clear stale cover immediately so we don't show old art on the
            // new track, then trigger a cover fetch + lyrics auto-fetch.
            if track_changed {
                self.np_cover.image = None;
                self.np_cover.stateful = None;
                // Invalidate pending fetch so stale responses cannot overwrite
                // the new track. Path is used alongside id to disambiguate
                // `id == 0` locally-inserted tracks.
                let cur_path = self.state.current_track.as_ref().map(|t| t.path.clone());
                self.np_cover.track_id = current_tid;
                self.np_cover.track_path = cur_path.clone();
                self.np_cover.pending_gen = None;
                // Fetch cover art when needed: for display (no_image_protocol
                // check) OR for reactive-theming palette extraction.
                let needs_cover = self.reactive_theme || !no_image_protocol();
                if needs_cover && let Some(tid) = current_tid {
                    let fetch_gen = self.next_cover_gen();
                    self.np_cover.pending_gen = Some(fetch_gen);
                    let client = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(Some(b64)) = client.art().cover(tid).await
                            && let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(&b64)
                        {
                            let _ =
                                ipc_tx.send(IpcResult::CoverArt(Some(bytes), Some(tid), fetch_gen));
                        }
                    });
                }
                // Auto-fetch lyrics on track change if enabled and (pane visible or auto-fetch enabled)
                let should_fetch_lyrics = self.auto_fetch_lyrics || self.lyrics.show;
                if should_fetch_lyrics {
                    let fetch_gen = self.next_lyrics_gen();
                    self.lyrics.current = None;
                    self.lyrics.pending_gen = Some(fetch_gen);
                    self.lyrics.fetching = true;
                    self.lyrics.scroll = 0;
                    let client = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    let tpath = self.state.current_track.as_ref().map(|t| t.path.clone());
                    let cur_tid = self.state.current_track.as_ref().map(|t| t.id);
                    tokio::spawn(async move {
                        let result = client
                            .lyrics()
                            .get(cur_tid.unwrap_or(0), tpath.as_deref())
                            .await;
                        match result {
                            Ok(lyrics) => {
                                let _ = ipc_tx.send(IpcResult::Lyrics(lyrics, fetch_gen));
                            }
                            Err(_) => {
                                let _ = ipc_tx.send(IpcResult::Lyrics(None, fetch_gen));
                            }
                        }
                    });
                }
            }

            while let Ok(result) = self.ipc_rx.try_recv() {
                match result {
                    IpcResult::RefreshDone(state, cover, cover_tid) => {
                        // Only apply if the new state is at least as recent as ours.
                        // A stale get_status() response can arrive after a PlaybackStarted
                        // event and overwrite the newer state if we don't guard this.
                        //
                        // Exception: a large backward version jump means the daemon
                        // restarted (its counter resets to 0).  The fresh snapshot is
                        // authoritative even though its version is lower than the local
                        // mirror, so it must not be dropped forever.
                        let restarted = state.version < self.state.version
                            && self.state.version.saturating_sub(state.version) > 1000;
                        if state.version >= self.state.version || restarted {
                            self.state = *state;
                            self.client.seed_clock_from_state(&self.state).await;
                            // Cover art is fetched on track-change events only.
                            // Do not clear track_id when a periodic RefreshDone
                            // carries no cover, or the track-change guard would
                            // re-download art (and re-fetch lyrics) every second.
                            // Robustness: verify cover_tid still matches current
                            // track (id+path) to avoid stale periodic cover
                            // overwriting a newer track's art.
                            if let Some(c) = cover {
                                let cur_tid = self.state.current_track.as_ref().map(|t| t.id);
                                let cur_path =
                                    self.state.current_track.as_ref().map(|t| t.path.clone());
                                let tid_matches = cover_tid == cur_tid;
                                let path_matches = match (&cover_tid, &cur_path) {
                                    (Some(_), Some(cp)) => {
                                        // If daemon supplied a tid, ensure path
                                        // consistency for `id == 0` disambiguation
                                        self.np_cover.track_path.as_deref() == Some(cp.as_str())
                                            || self.np_cover.track_path.is_none()
                                    }
                                    (None, None) => true,
                                    _ => tid_matches,
                                };
                                if tid_matches && path_matches {
                                    if self.reactive_theme {
                                        let tx = self.ipc_tx.clone();
                                        self.request_reactive_palette(&c, tx);
                                    }
                                    self.np_cover.image = Some(c);
                                    self.np_cover.track_id = cover_tid;
                                    if !no_image_protocol() {
                                        self.cover_sync();
                                    } else {
                                        self.cover_art_dirty = true;
                                    }
                                }
                            }
                        }
                    }
                    IpcResult::CoverArt(cover, cover_tid, fetch_gen) => {
                        // Generation + id+path guard: stale Covers for fast
                        // skips (A->B->C) are dropped if fetch_gen mismatches or tid
                        // no longer equals current track. Covers `id == 0`
                        // reuse (queued Spotify/YouTube) via generation.
                        let Some(pending) = self.np_cover.pending_gen else {
                            // No pending fetch — likely track changed and cleared,
                            // drop stale.
                            continue;
                        };
                        if fetch_gen != pending {
                            continue;
                        }
                        let cur_tid = self.state.current_track.as_ref().map(|t| t.id);
                        if cover_tid != cur_tid {
                            continue;
                        }
                        // Path check for `id == 0` disambiguation
                        if let (Some(_), Some(cur_path)) = (
                            &cover_tid,
                            self.state.current_track.as_ref().map(|t| &t.path),
                        ) && let Some(pending_path) = self.np_cover.track_path.as_ref()
                            && pending_path != cur_path
                        {
                            continue;
                        }
                        if let Some(c) = cover.as_ref()
                            && self.reactive_theme
                        {
                            let tx = self.ipc_tx.clone();
                            self.request_reactive_palette(c, tx);
                        }
                        self.np_cover.pending_gen = None;
                        self.np_cover.image = cover;
                        self.np_cover.track_id = cover_tid;
                        if !no_image_protocol() {
                            self.cover_sync();
                        }
                        self.cover_art_dirty = true;
                    }
                    IpcResult::ReactivePalette(pal) => {
                        if self.reactive_theme {
                            self.reactive_palette = pal;
                            self.apply_reactive();
                        }
                    }
                    IpcResult::SubsonicStatus(st) => self.subsonic.status = st,
                    IpcResult::SubsonicSearch(res) => {
                        self.subsonic.search_results = res;
                        self.subsonic.search_pending = false;
                    }
                    IpcResult::SubsonicAlbums(a) => {
                        self.subsonic.albums = a;
                        self.subsonic.albums_pending = false;
                    }
                    IpcResult::SubsonicAlbumTracks(t) => {
                        self.subsonic.album_tracks = t;
                    }
                    IpcResult::SubsonicCover(data) => self.subsonic.cover_preview = data,
                    IpcResult::PodcastStatus(st) => self.podcast.status = st,
                    IpcResult::PodcastFeeds(feeds) => {
                        self.podcast.feeds = feeds;
                        self.podcast.feeds_pending = false;
                    }
                    IpcResult::PodcastEpisodes(eps) => {
                        self.podcast.episodes = eps;
                    }
                    IpcResult::RadioSearch(stations) => {
                        self.radio.search = stations;
                        self.radio.search_pending = false;
                    }
                    IpcResult::RadioTop(stations) => {
                        self.radio.top = stations;
                        self.radio.top_pending = false;
                    }
                    IpcResult::RadioTags(tags) => {
                        self.radio.browse_tags = tags;
                        self.radio.browse_pending = false;
                    }
                    IpcResult::RadioCountries(countries) => {
                        self.radio.browse_countries = countries;
                        self.radio.browse_pending = false;
                    }
                    IpcResult::RadioBrowseStations(stations) => {
                        self.radio.browse_stations = stations;
                        self.radio.browse_stations_pending = false;
                    }
                    IpcResult::LastfmStatus(st) => {
                        let was_ready = self.setup.lastfm_status.as_ref().is_some_and(|s| s.ready);
                        self.setup.lastfm_status = st;
                        let now_ready = self.setup.lastfm_status.as_ref().is_some_and(|s| s.ready);
                        if now_ready
                            && !was_ready
                            && self
                                .pickers
                                .top()
                                .is_some_and(|o| o.id == PickerId::LastfmAuth)
                        {
                            self.setup.lastfm_pending = false;
                            self.setup.lastfm_auth_url = None;
                            self.setup.lastfm_error = None;
                            self.notify_titled(
                                "Last.fm",
                                "Last.fm linked — scrobbling is now active",
                                NotificationKind::Success,
                                false,
                                NotifType::Lastfm,
                            );
                            self.close_top_picker_with_cleanup();
                        }
                    }
                    IpcResult::LastfmAuthUrl(url) => {
                        self.setup.lastfm_auth_url = Some(url);
                        self.setup.lastfm_pending = true;
                        let c = self.client.clone();
                        let ipc_tx = self.ipc_tx.clone();
                        tokio::spawn(async move {
                            match capture_lastfm_token_loopback().await {
                                Ok(token) if token.is_empty() => {
                                    self_err(&ipc_tx, "no Last.fm token provided".to_string());
                                }
                                Ok(token) => match c.lastfm().authenticate(token.trim()).await {
                                    Ok(()) => match c.lastfm().status().await {
                                        Ok(st) => {
                                            let _ = ipc_tx.send(IpcResult::LastfmStatus(Some(st)));
                                        }
                                        Err(e) => {
                                            self_err(&ipc_tx, format!("last.fm status failed: {e}"))
                                        }
                                    },
                                    Err(e) => {
                                        let _ = ipc_tx.send(IpcResult::LastfmAuthError(format!(
                                            "Last.fm authorize failed: {e}"
                                        )));
                                    }
                                },
                                Err(e) => {
                                    let _ = ipc_tx.send(IpcResult::LastfmAuthError(format!(
                                        "Last.fm callback failed: {e}"
                                    )));
                                }
                            }
                        });
                    }
                    IpcResult::LastfmAuthError(e) => {
                        self.setup.lastfm_pending = false;
                        self.setup.lastfm_error = Some(e.clone());
                        self.notify_titled(
                            "Last.fm",
                            e,
                            NotificationKind::Error,
                            false,
                            NotifType::Lastfm,
                        );
                    }
                    IpcResult::LibraryTracks(tracks) => self.tracks_cache = tracks,
                    IpcResult::Playlists(playlists) => self.playlist_cache = playlists,
                    IpcResult::PlaylistCreated(id, _name) => {
                        self.pending_playlist_id = Some(id);
                        self.selected_playlist_track_ids.clear();
                        self.pickers.open(PickerId::PlaylistTrackSelect);
                        self.pending_playlist_track_ids = vec![id];
                    }
                    IpcResult::PlaylistTracks(tracks) => self.playlist_tracks_cache = tracks,
                    IpcResult::Queue(tracks, cursor) => {
                        let cursor_changed = self.queue.cursor != cursor;
                        self.queue.cache = tracks.clone();
                        self.queue.cursor = cursor;
                        if cursor_changed {
                            self.queue.preview_cover = None;
                            self.queue.preview_cover_stateful = None;
                        }
                        if self.queue.cache.is_empty() && self.state.current_track.is_none() {
                            self.reset_library_view(self.library_category, None);
                        }
                    }
                    IpcResult::YtResults(query, results) => {
                        // Apply results only when they belong to the query the
                        // picker currently shows; stale results from a
                        // superseded search are dropped.
                        let current = self
                            .pickers
                            .top()
                            .filter(|t| t.id == PickerId::YTSearch)
                            .map(|t| t.query.clone());
                        if current.as_deref() == Some(query.as_str()) {
                            // Interleave: 1 playlist for every 3 tracks
                            self.yt_results_cache = Self::interleave_yt_results(results);
                            self.yt_search_loading = false;
                        }
                    }
                    IpcResult::Notification(title, msg, kind, ntype) => {
                        // Petty flow acknowledgements (add/remove playlist,
                        // playlist creation hand-off, cache clears) surface in
                        // the footer or history instead of floating cards that
                        // interrupt the view.
                        let trivial = (title == "Playlist"
                            && (msg.starts_with("Tracks added to playlist")
                                || msg.starts_with("Removed from playlist")
                                || msg.starts_with("Created ")))
                            || title == "Cache";
                        self.notify_typed(&title, msg, kind, trivial, ntype);
                    }
                    IpcResult::Error(e) => {
                        self.notify(e, NotificationKind::Error);
                    }
                    IpcResult::YtDownloadProgress {
                        id,
                        url,
                        title,
                        progress,
                        status,
                        file_path,
                        downloaded_bytes,
                        total_bytes,
                        rate_bytes_per_sec,
                        eta_secs,
                    } => {
                        let terminal =
                            matches!(status.as_str(), "completed" | "failed" | "cancelled");
                        if terminal {
                            self.downloads.remove(&id);
                        } else {
                            let last = self.downloads.get(&id).cloned();
                            let smooth = if let Some(last) = last
                                && last.percent > 0.0
                                && progress > last.percent
                            {
                                // EMA with ~0.6 inertia per update (~250ms) so
                                // the footer bar glides instead of jittering.
                                last.percent + (progress - last.percent) * 0.4
                            } else {
                                progress
                            };
                            self.downloads.insert(
                                id,
                                DownloadProgressView {
                                    url,
                                    title,
                                    status,
                                    file_path,
                                    percent: smooth,
                                    downloaded_bytes,
                                    total_bytes,
                                    rate_bytes_per_sec,
                                    eta_secs,
                                    updated_at: std::time::Instant::now(),
                                },
                            );
                        }
                    }
                    IpcResult::PopupCoverArt(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.track_popup_track_id == Some(track_id)
                            && self.last_popup_cover_fetch_gen == Some(fetch_gen)
                        {
                            self.track_popup_cover = cover;
                            self.popup_cover_sync();
                        }
                    }
                    IpcResult::UpNextCover(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.upnext.as_ref().is_some_and(|u| {
                                u.cover_fetch_id == Some(track_id)
                                    && u.track.id == track_id
                                    && u.cover_fetch_gen == Some(fetch_gen)
                            })
                            && let Some(u) = self.upnext.as_mut()
                        {
                            u.cover = cover;
                            self.upnext_cover_sync();
                        }
                    }
                    IpcResult::QueuePreviewCover(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.queue.last_preview_cover_fetch_id == Some(track_id)
                            && self.queue.last_preview_cover_fetch_gen == Some(fetch_gen)
                        {
                            self.queue.preview_cover = cover;
                            self.queue_preview_cover_sync();
                            // The cover arrived on the IPC event loop; force a
                            // redraw this frame so the picker shows it without
                            // waiting for a coincidental render trigger.
                            self.cover_art_dirty = true;
                            if self.queue.preview_cover.is_some() {
                                self.queue.preview_cover_fail_until = None;
                            } else {
                                // Failed or empty: release the guard so a later
                                // preview retries, and throttle the refetch.
                                self.queue.last_preview_cover_fetch_gen = None;
                                self.queue.preview_cover_fail_until = Some((
                                    track_id,
                                    std::time::Instant::now() + Duration::from_secs(30),
                                ));
                            }
                        }
                    }
                    IpcResult::PickerPreviewCover(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.last_picker_preview_fetch_id == Some(track_id)
                            && self.last_picker_preview_fetch_gen == Some(fetch_gen)
                        {
                            self.picker_preview_cover = cover;
                            self.picker_preview_sync();
                            self.cover_art_dirty = true;
                        }
                    }
                    IpcResult::MetadataCoverArt(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.metadata.edit_track_id == Some(track_id)
                            && self.metadata.cover_fetch_gen == Some(fetch_gen)
                        {
                            self.metadata.cover = cover;
                            self.metadata_cover_sync();
                            self.metadata.cover_dirty = true;
                        }
                    }
                    IpcResult::ArtistCoverArt(cover, artist, fetch_gen) => {
                        if !no_image_protocol()
                            && self.last_artist_cover_fetch.as_deref() == Some(&artist)
                            && self.last_artist_cover_fetch_gen == Some(fetch_gen)
                        {
                            self.artist_cover = cover;
                            self.artist_cover_sync();
                        }
                    }
                    IpcResult::SpotifyPreviewCover(cover, url, fetch_gen) => {
                        if !no_image_protocol()
                            && self.spotify.last_preview_fetch.as_deref() == Some(&url)
                            && self.spotify.last_preview_fetch_gen == Some(fetch_gen)
                        {
                            self.spotify.preview_cover = cover;
                            self.spotify_preview_sync();
                        }
                    }
                    IpcResult::CoverPicker(picker) => {
                        self.np_cover.picker = picker;
                        // Rebuild all active StatefulProtocols with the new
                        // picker geometry. Previously they retained the old
                        // picker's resize state, causing cropped / stale sizes
                        // after terminal resize or image-protocol renegotiation
                        //.
                        self.cover_sync();
                        self.popup_cover_sync();
                        self.upnext_cover_sync();
                        self.queue_preview_cover_sync();
                        self.picker_preview_sync();
                        self.artist_cover_sync();
                        self.spotify_preview_sync();
                        self.metadata_cover_sync();
                    }
                    IpcResult::Lyrics(lyrics, lyrics_gen) => {
                        if Some(lyrics_gen) != self.lyrics.pending_gen {
                            // Stale: the track changed while this fetch was in
                            // flight, so the lines belong to the previous song.
                            // Drop rather than flash the wrong lyrics.
                        } else {
                            self.lyrics.pending_gen = None;
                            self.lyrics.current = lyrics;
                            self.lyrics.fetching = false;
                            // Snap to the lyric line matching the current
                            // playback position so opening lyrics mid-track
                            // doesn't start with the first line highlighted.
                            self.lyrics.scroll = self.current_lyric_index();
                            // Show "No lyrics found" if lyrics fetch returned None
                            if self.lyrics.current.is_none() {
                                self.lyrics.current = Some(LrcData {
                                    title: None,
                                    artist: None,
                                    album: None,
                                    lines: vec![LrcLine {
                                        timestamp: 0.0,
                                        text: "No lyrics found".to_string(),
                                    }],
                                });
                            }
                        }
                    }
                    IpcResult::HealthReport(report) => {
                        self.health_report = Some(report);
                        self.show_health_panel = std::mem::take(&mut self.show_health_on_report);
                    }
                    IpcResult::SpotifyStatus(s) => self.spotify.status = Some(s),
                    IpcResult::SpotifyOauthUrl(url) => {
                        self.spotify.oauth_url = Some(url);
                        self.spotify.oauth_error = None;
                    }
                    IpcResult::SpotifyOauthError(e) => {
                        let e = e.to_string();
                        self.spotify.oauth_error = Some(e.clone());
                        self.spotify.oauth_pending = false;
                        // Keep the picker open so the error and URL stay
                        // visible; Esc closes it (clearing the state below).
                        self.notify_titled(
                            "Spotify",
                            e,
                            NotificationKind::Error,
                            false,
                            NotifType::Spotify,
                        );
                    }
                    IpcResult::SpotifyPlaylists(p) => self.spotify.playlists = p,
                    IpcResult::SpotifyTracks(t) => self.spotify.playlist_tracks_cache = t,
                    IpcResult::SpotifySearchWebResults(seq, tracks) => {
                        if seq != self.spotify.web_seq {
                            // Stale: a newer query superseded this in-flight
                            // response, so its rows would be for the wrong
                            // search. Drop rather than flash wrong results.
                            continue;
                        }
                        for track in tracks {
                            self.spotify.search_results.push((
                                "web".into(),
                                "Spotify".into(),
                                track,
                            ));
                        }
                    }
                }
            }

            while let Ok(cmd) = self.high_pri_cmd_rx.try_recv() {
                self.handle_command(cmd);
            }
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd);
            }

            // YT search debounce: auto-search 500ms after last keystroke
            let now = std::time::Instant::now();
            if let Some(deadline) = self.yt_search_debounce
                && now >= deadline
            {
                self.yt_search_debounce = None;
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::YTSearch
                    && !top.query.is_empty()
                {
                    let q = top.query.clone();
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::YtSearch(q)).await;
                }
            }

            // Spotify web-search debounce: auto-search 500ms after the last
            // keystroke, mirroring the YT search behaviour above.
            let now = std::time::Instant::now();
            if let Some(deadline) = self.spotify.search_debounce
                && now >= deadline
            {
                self.spotify.search_debounce = None;
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::SpotifySearch
                    && !top.query.is_empty()
                {
                    self.search_spotify();
                }
            }

            // YT search polling: while a search is in flight, poll for results
            // so the picker populates without requiring an extra Enter.  The
            // daemon runs the search on a background task; polls return the
            // results once it completes.
            if self.yt_search_loading
                && self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::YTSearch)
            {
                if self.yt_search_poll_deadline.is_none() {
                    self.yt_search_poll_deadline = Some(now + Duration::from_millis(500));
                }
                if now >= self.yt_search_poll_deadline.unwrap_or(now) {
                    self.yt_search_poll_deadline = Some(now + Duration::from_millis(700));
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::RefreshYt).await;
                }
            } else {
                self.yt_search_poll_deadline = None;
            }

            // Detect state changes
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            if current_tid != self.prev_track_id {
                self.prev_track_id = current_tid;
            }
            if self.state.status != self.prev_status {
                self.prev_status = self.state.status;
            }
            // Volume changes: update the previous volume tracker without triggering an animation.
            if self.state.volume != self.prev_volume {
                self.prev_volume = self.state.volume;
            }
            if self.np_cover.track_id != self.prev_cover_id {
                self.prev_cover_id = self.np_cover.track_id;
            }

            let mut raw_pos = self.client.estimated_position().await;
            // Monotonic guard: prevent large backward jumps from clock skew.
            // Allow at most 0.5s of regression to avoid visible stutter.
            // Skipped right after a seek, otherwise a backward seek gets clamped
            // and the lyric highlight never re-syncs to the new position.
            let seeking = self
                .seek_pending
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1200));
            if seeking {
                // A real (in-track) seek target has landed: drop the window so
                // the guard resumes once playback continues past it.
                if self.state.duration > 0.0 && self.raw_position <= self.state.duration {
                    self.seek_pending = None;
                }
            } else {
                raw_pos = raw_pos.max(self.display_position - 0.5);
            }
            // Lyric matching uses the raw (guard-only) position so the active
            // verse switches at the right timestamp; the smoothing below only
            // glides the progress bar. While a seek is being accumulated the
            // daemon hasn't seeked yet, so prefer the optimistic local estimate
            // rather than the still-stale daemon position.
            if self.seek_cmd_accum.is_none() {
                self.raw_position = raw_pos;
            }
            let now = std::time::Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.25);
            self.last_frame = now;
            if self.seek_cmd_accum.is_some() {
                // Holding a seek key: hold the optimistic position so the bar
                // and highlight track the accumulated target, not the stale
                // daemon position.
                self.display_position = self.raw_position;
            } else {
                self.display_position +=
                    (raw_pos - self.display_position) * (1.0 - (-dt / 0.08).exp());
            }
            // Debounced daemon seek dispatch for long-press seeking.
            self.ensure_seek_flush();
            let bar_dur = if self.state.duration > 0.0 {
                self.state.duration
            } else {
                self.state
                    .current_track
                    .as_ref()
                    .map(|t| t.duration)
                    .unwrap_or(0.0)
            };
            let bar_target = if bar_dur > 0.0 {
                self.display_position / bar_dur
            } else {
                0.0
            };
            self.progress_smoother.smooth(bar_target, dt);

            // Auto-resume lyric auto-follow: after a manual scroll, once
            // playback reaches the line the user scrolled to, re-enable
            // auto-follow so the highlight can't stay frozen for the rest of
            // the track.  Reading ahead still holds until the audio catches up.
            if self.lyrics.manual_scroll
                && self.lyrics.show
                && self.lyrics.current.is_some()
                && self.current_lyric_index() >= self.lyrics.scroll
            {
                self.lyrics.manual_scroll = false;
            }

            // Auto-scroll lyrics to current playback position.  Not gated on
            // `status == Playing` so a mirrored-status desync (e.g. after a
            // daemon restart) can't freeze the highlight at the first line.
            if !self.lyrics.manual_scroll && self.lyrics.show && self.lyrics.current.is_some() {
                self.lyrics.scroll = self.current_lyric_index();
            }

            // Dirty-render: skip redraw if position hasn't changed meaningfully
            // to reduce CPU usage.  Always render every 10th frame as a safety net.
            let frame_count = self.frame_count.wrapping_add(1);
            self.frame_count = frame_count;

            // Advance progress whip scanner (Knight Rider style)
            const SCANNER_WIDTH: i32 = 12;
            const SCANNER_HOLD: i32 = 3;
            if self.scanner_hold > 0 {
                self.scanner_hold -= 1;
            } else {
                self.scanner_pos += self.scanner_dir;
                if self.scanner_pos >= SCANNER_WIDTH - 1 {
                    self.scanner_dir = -1;
                    self.scanner_hold = SCANNER_HOLD;
                } else if self.scanner_pos <= 0 {
                    self.scanner_dir = 1;
                    self.scanner_hold = SCANNER_HOLD;
                }
            }

            // Blink cursor every 8 frames
            if frame_count.is_multiple_of(8) {
                self.cursor_blink = !self.cursor_blink;
            }

            // Hot-reload config every 120 frames (~2s at 60fps)
            if frame_count.is_multiple_of(120) {
                self.check_config_reload();
            }

            let pos_changed = (self.display_position - self.last_display_position).abs() >= 0.05;
            // Advance title scroll animations (every 3rd frame)
            if frame_count.is_multiple_of(3) {
                self.footer_title_scroll = self.footer_title_scroll.wrapping_add(1);
                self.np_title_scroll = self.np_title_scroll.wrapping_add(1);
            }

            let playing = self.state.status == PlaybackStatus::Playing;
            let force_render = pos_changed
                || (playing && frame_count.is_multiple_of(2))
                // Visualizer animates continuously (idle wave included).
                || self.visualizer.is_enabled()
                || !self.notifications.is_empty()
                || frame_count.is_multiple_of(10)
                || self.cover_art_dirty
                || self.metadata.cover_dirty
                || self.track_anim_trigger
                || self.anim_fx.is_running();
            self.cover_art_dirty = false;
            self.metadata.cover_dirty = false;
            self.last_display_position = self.display_position;

            if force_render {
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                self.terminal_cols = cols;
                self.terminal_rows = rows;
                if cols < 20 || rows < 6 {
                    let _ = terminal.draw(|f| {
                        let msg = Paragraph::new("Terminal too small (min 20x6)")
                            .alignment(Alignment::Center);
                        f.render_widget(msg, f.area());
                    });
                } else if terminal.draw(|f| ui::render(f, &mut self)).is_ok() {
                    self.footer_cache.suppress_refresh = false;
                }
            }

            if event::poll(Duration::from_millis(16)).unwrap_or(false)
                && let Ok(ev) = event::read()
                && !self.handle_terminal_event(ev).await
            {
                break 'outer;
            }

            // Handle Ctrl+Z suspend: leave terminal, SIGTSTP, re-init on resume
            if self.pending_suspend {
                self.pending_suspend = false;
                let _ = crossterm::terminal::disable_raw_mode();
                let mut stdout = std::io::stdout();
                let _ = crossterm::execute!(
                    stdout,
                    crossterm::terminal::LeaveAlternateScreen,
                    crossterm::event::DisableBracketedPaste,
                    crossterm::event::DisableMouseCapture
                );
                // Suspend: this blocks until SIGCONT
                unsafe {
                    libc::raise(libc::SIGTSTP);
                }
                // Resumed: re-init terminal
                crossterm::terminal::enable_raw_mode()?;
                let mut stdout = std::io::stdout();
                crossterm::execute!(
                    stdout,
                    crossterm::terminal::EnterAlternateScreen,
                    crossterm::event::EnableBracketedPaste,
                    crossterm::event::EnableMouseCapture
                )?;
                terminal.clear()?;
            }
        }
        Ok(())
    }

    /// Index of the time-synced lyric line for the current playback position.
    /// Untimed lines (timestamp < 0) are skipped for matching but keep their
    /// index so the highlight tracks timed lines correctly.
    pub fn current_lyric_index(&self) -> usize {
        let Some(ref lyrics) = self.lyrics.current else {
            return 0;
        };
        lyric_index_at(&lyrics.lines, self.raw_position)
    }

    /// Cycle pane focus with Tab/Shift-Tab.  With lyrics open this walks
    /// left pane → right pane → lyrics pane (or the reverse for Shift-Tab);
    /// otherwise it toggles the left/right panes.  Leaving the lyrics pane
    /// re-enables lyric auto-follow.
    fn cycle_pane_focus(&mut self, forward: bool) {
        if self.lyrics.show {
            let (lib, lyr) =
                cycle_library_focus(self.library_pane_focus, self.lyrics.pane_focus, forward);
            self.library_pane_focus = lib;
            self.lyrics.pane_focus = lyr;
            if !lyr {
                self.lyrics.manual_scroll = false;
            }
        } else {
            self.library_pane_focus = !self.library_pane_focus;
        }
    }

    pub fn notify(&mut self, message: impl Into<String>, kind: NotificationKind) {
        let title = match kind {
            NotificationKind::Info | NotificationKind::Warning => "System",
            NotificationKind::Success => "Success",
            NotificationKind::Error => "Error",
        };
        self.notify_typed(title, message, kind, false, NotifType::System);
    }

    pub fn notify_titled(
        &mut self,
        title: &str,
        message: impl Into<String>,
        kind: NotificationKind,
        trivial: bool,
        ntype: NotifType,
    ) {
        self.notify_typed(title, message, kind, trivial, ntype);
    }

    /// Core emitter: record history and surface `message` per the category's
    /// configured mode. History is always kept so nothing is lost when a
    /// category is muted or demoted to the footer.
    pub fn notify_typed(
        &mut self,
        title: &str,
        message: impl Into<String>,
        kind: NotificationKind,
        trivial: bool,
        ntype: NotifType,
    ) {
        let message = message.into();
        self.notification_history.insert(
            0,
            NotificationRecord {
                title: title.to_string(),
                message: message.clone(),
                kind: kind.clone(),
                at: std::time::Instant::now(),
            },
        );
        self.notification_history.truncate(50);
        let mode = self
            .notification_modes
            .get(&ntype)
            .copied()
            .unwrap_or(NotifMode::Floating);
        match mode {
            NotifMode::Floating => {
                let expires_at = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                self.notifications.push(Notification {
                    title: title.to_string(),
                    message,
                    kind,
                    expires_at,
                    slide_direction: SlideDirection::Right,
                    is_volume: false,
                    volume_value: 0,
                    animation_progress: 0.0,
                    trivial,
                });
            }
            NotifMode::Footer => {
                self.notify_footer(format!("{title}: {message}"));
            }
            NotifMode::Off => {
                // History only.
            }
        }
    }

    pub fn notify_footer(&mut self, message: impl Into<String>) {
        self.footer_notification = Some((
            message.into(),
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        ));
    }

    pub fn notify_volume(&mut self, volume: u8) {
        // Volume is a Playback-class notification: honor its configured mode.
        let mode = self
            .notification_modes
            .get(&NotifType::Playback)
            .copied()
            .unwrap_or(NotifMode::Floating);
        if mode == NotifMode::Off {
            return;
        }
        let now = std::time::Instant::now();
        if mode == NotifMode::Footer {
            self.notify_footer(format!("Vol {volume}"));
            return;
        }
        if let Some(existing) = self
            .notifications
            .iter_mut()
            .find(|n| n.is_volume && n.expires_at > now)
        {
            existing.volume_value = volume;
            existing.expires_at = now + std::time::Duration::from_millis(1500);
            return;
        }
        let expires_at = now + std::time::Duration::from_millis(1500);
        self.notifications.push(Notification {
            title: "Volume".to_string(),
            message: format!("Vol {}", volume),
            kind: NotificationKind::Info,
            expires_at,
            slide_direction: SlideDirection::Right,
            is_volume: true,
            volume_value: volume,
            animation_progress: 0.0,
            trivial: true,
        });
    }

    pub fn start_upnext(&mut self, track: TrackInfo) {
        let total_secs = self.crossfade_duration as f64 + 3.0;
        let fetch_gen = if no_image_protocol() {
            None
        } else {
            Some(self.next_cover_gen())
        };
        let fetch_id = if fetch_gen.is_some() {
            Some(track.id)
        } else {
            None
        };
        self.upnext = Some(UpNextNotif {
            track: track.clone(),
            cover: None,
            cover_stateful: None,
            started_at: std::time::Instant::now(),
            total_secs,
            cover_fetch_id: fetch_id,
            cover_fetch_gen: fetch_gen,
        });
        let Some(fetch_gen) = fetch_gen else {
            return;
        };
        let tid = track.id;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::UpNextCover(Some(bytes), tid, fetch_gen));
            }
        });
    }

    /// Kind of item the library track-info block is currently describing,
    /// derived from the active list and drill-down state.
    pub fn track_info_kind(&self) -> TrackInfoKind {
        if self.browse_detail.is_some() {
            if self.library_category == 5 {
                return TrackInfoKind::SpotifyTrack;
            }
            return TrackInfoKind::Track;
        }
        match self.library_category {
            2 => TrackInfoKind::Album,
            3 => TrackInfoKind::Artist,
            4 => TrackInfoKind::Playlist,
            5 => TrackInfoKind::SpotifyPlaylist,
            _ => TrackInfoKind::Track,
        }
    }

    /// Update the track popup to describe the currently selected row in the
    /// library list, context aware of the active list type.
    /// Tracks, albums and artists resolve a representative track so cover art
    /// can be fetched; playlist and Spotify rows show meta only.
    pub fn update_track_popup(&mut self) {
        let kind = self.track_info_kind();
        let maybe_track: Option<(i64, String)> = match kind {
            TrackInfoKind::Track => {
                let filtered = self.filtered_tracks();
                let pos = self.list_pos();
                filtered.get(pos).map(|t| (t.id, t.path.clone()))
            }
            TrackInfoKind::Album => {
                let albums = self.unique_albums();
                let pos = self.list_pos();
                albums.get(pos).and_then(|(name, _)| {
                    self.tracks_cache
                        .iter()
                        .find(|t| {
                            let album: &str = if t.album.is_empty() {
                                "Unknown Album"
                            } else {
                                &t.album
                            };
                            album == name
                        })
                        .map(|t| (t.id, t.path.clone()))
                })
            }
            TrackInfoKind::Artist => {
                let artists = self.unique_artists();
                let pos = self.list_pos();
                artists.get(pos).and_then(|(name, _)| {
                    self.tracks_cache
                        .iter()
                        .find(|t| {
                            let artist: &str = if t.artist.is_empty() {
                                "Unknown Artist"
                            } else {
                                &t.artist
                            };
                            artist == name
                        })
                        .map(|t| (t.id, t.path.clone()))
                })
            }
            // Playlist and Spotify rows never resolve a cover; the block still
            // describes the selected row (playlist name / spotify track).
            TrackInfoKind::Playlist
            | TrackInfoKind::SpotifyPlaylist
            | TrackInfoKind::SpotifyTrack => None,
        };

        let valid = match kind {
            TrackInfoKind::Playlist => self.list_pos() < self.playlist_cache.len(),
            TrackInfoKind::SpotifyPlaylist => self.list_pos() < self.spotify.playlists.len(),
            TrackInfoKind::SpotifyTrack => self.selected_spotify_track().is_some(),
            _ => maybe_track.is_some(),
        };

        self.track_popup_visible = valid;
        if !valid {
            self.clear_popup_cover();
            return;
        }

        let Some((tid, path)) = maybe_track else {
            self.clear_popup_cover();
            return;
        };
        self.track_popup_track_id = Some(tid);

        let current_is_selected = self
            .state
            .current_track
            .as_ref()
            .is_some_and(|t| t.path == path);
        if current_is_selected {
            // Robust fallback: if current track's art is still pending (cleared
            // on track change), fall through to fetch rather than showing blank
            //. Only reuse when we actually have bytes.
            if let Some(cover) = self.np_cover.image.clone() {
                self.track_popup_cover = Some(cover);
                self.popup_cover_sync();
                self.last_popup_cover_fetch_id = None;
                self.last_popup_cover_fetch_gen = None;
                return;
            }
            // else fall through to fetch below
        }
        // Generation-guarded fetch: `id == 0` reuse is safe via fetch_gen.
        let already_pending = self.last_popup_cover_fetch_id == Some(tid)
            && self.last_popup_cover_fetch_gen.is_some()
            && !no_image_protocol();
        if already_pending {
            return;
        }
        if no_image_protocol() {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.last_popup_cover_fetch_id = Some(tid);
        self.last_popup_cover_fetch_gen = Some(fetch_gen);
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::PopupCoverArt(Some(bytes), tid, fetch_gen));
            }
        });
    }

    /// Dismiss the track popup.
    pub fn dismiss_track_popup(&mut self) {
        self.track_popup_visible = false;
        self.clear_popup_cover();
    }

    /// Fetch cover art for the highlighted SearchLibrary picker row so the
    /// preview window can render the actual album art as ASCII.
    pub fn update_picker_preview(&mut self) {
        let Some(top) = self.pickers.top() else {
            // Robust: invalidate pending fetch_gen so close/reopen does not retain stale key
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.last_picker_preview_fetch_id = None;
            self.last_picker_preview_fetch_gen = None;
            return;
        };
        if top.id != PickerId::SearchLibrary {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.last_picker_preview_fetch_id = None;
            self.last_picker_preview_fetch_gen = None;
            return;
        }
        let picks = self.search_library_picks();
        if picks.is_empty() {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.last_picker_preview_fetch_id = None;
            self.last_picker_preview_fetch_gen = None;
            return;
        }
        let sel = top.selected.min(picks.len() - 1);
        let LibraryPick::Track(i) = &picks[sel] else {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.last_picker_preview_fetch_id = None;
            self.last_picker_preview_fetch_gen = None;
            return;
        };
        let tid = self.tracks_cache[*i].id;
        // Generation-guarded dedup: id reuse (id==0) cannot block new fetches.
        // Only skip when both id and generation match current pending.
        if self.last_picker_preview_fetch_id == Some(tid)
            && self.last_picker_preview_fetch_gen.is_some()
        {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.last_picker_preview_fetch_id = Some(tid);
        self.last_picker_preview_fetch_gen = Some(fetch_gen);
        self.picker_preview_cover = None;
        self.picker_preview_stateful = None;
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::PickerPreviewCover(Some(bytes), tid, fetch_gen));
            }
        });
    }

    pub fn update_artist_cover(&mut self) {
        let Some(top) = self.pickers.top() else {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.last_artist_cover_fetch = None;
            self.last_artist_cover_fetch_gen = None;
            return;
        };
        if top.id != PickerId::SearchLibrary {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.last_artist_cover_fetch = None;
            self.last_artist_cover_fetch_gen = None;
            return;
        }
        let picks = self.search_library_picks();
        if picks.is_empty() {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.last_artist_cover_fetch = None;
            self.last_artist_cover_fetch_gen = None;
            return;
        }
        let sel = top.selected.min(picks.len() - 1);
        let LibraryPick::Artist(name) = &picks[sel] else {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.last_artist_cover_fetch = None;
            self.last_artist_cover_fetch_gen = None;
            return;
        };
        if self.last_artist_cover_fetch.as_deref() == Some(name.as_str())
            && self.last_artist_cover_fetch_gen.is_some()
        {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.last_artist_cover_fetch = Some(name.clone());
        self.last_artist_cover_fetch_gen = Some(fetch_gen);
        self.artist_cover = None;
        self.artist_cover_stateful = None;
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let artist = name.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().artist_cover(artist.clone()).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::ArtistCoverArt(Some(bytes), artist, fetch_gen));
            }
        });
    }

    /// Fuzzy-finder rows for the SearchLibrary picker, filtered by the
    /// picker's active `PickerSource` and query.
    pub fn search_library_picks(&self) -> Vec<LibraryPick> {
        let Some(top) = self.pickers.top() else {
            return Vec::new();
        };
        let q = top.query.to_lowercase();
        let mut picks = Vec::new();
        match top.source {
            PickerSource::Tracks | PickerSource::All => {
                for (i, t) in self.tracks_cache.iter().enumerate() {
                    if q.is_empty()
                        || t.title.to_lowercase().contains(&q)
                        || t.artist.to_lowercase().contains(&q)
                        || t.album.to_lowercase().contains(&q)
                    {
                        picks.push(LibraryPick::Track(i));
                    }
                }
            }
            _ => {}
        }
        if matches!(top.source, PickerSource::Artists | PickerSource::All) {
            let mut seen = std::collections::HashSet::new();
            for t in &self.tracks_cache {
                if t.artist.is_empty() || !seen.insert(t.artist.to_lowercase()) {
                    continue;
                }
                if q.is_empty() || t.artist.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Artist(t.artist.clone()));
                }
            }
        }
        if matches!(top.source, PickerSource::Albums | PickerSource::All) {
            let mut seen = std::collections::HashSet::new();
            for t in &self.tracks_cache {
                if t.album.is_empty() || !seen.insert(t.album.to_lowercase()) {
                    continue;
                }
                if q.is_empty() || t.album.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Album(t.album.clone()));
                }
            }
        }
        if matches!(top.source, PickerSource::Playlists | PickerSource::All) {
            for (i, p) in self.playlist_cache.iter().enumerate() {
                if q.is_empty() || p.name.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Playlist(i));
                }
            }
        }
        picks
    }

    pub fn search_spotify(&mut self) {
        let q = self
            .pickers
            .top()
            .map_or(String::new(), |o| o.query.to_lowercase());
        self.spotify.search_results.clear();
        self.spotify.last_preview_fetch = None;
        self.spotify.last_preview_fetch_gen = None;
        self.spotify.preview_cover = None;
        self.spotify.preview_cover_stateful = None;
        if q.is_empty() {
            return;
        }
        for pl in &self.spotify.playlists {
            for track in &pl.tracks {
                if track.name.to_lowercase().contains(&q)
                    || track.artists.to_lowercase().contains(&q)
                    || track
                        .album
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
                {
                    self.spotify.search_results.push((
                        pl.id.clone(),
                        pl.name.clone(),
                        track.clone(),
                    ));
                }
            }
        }
        let query = self
            .pickers
            .top()
            .map_or(String::new(), |o| o.query.clone());
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let seq = self.spotify.web_seq;
        tokio::spawn(async move {
            match c.spotify().search_web(&query).await {
                Ok(tracks) => {
                    let _ = ipc_tx.send(IpcResult::SpotifySearchWebResults(seq, tracks));
                }
                Err(e) => {
                    let _ =
                        ipc_tx.send(IpcResult::Error(format!("Spotify Web search failed: {e}")));
                }
            }
        });
    }

    /// Kick off data fetches right after a remote-service picker opens.
    pub fn on_picker_opened(&mut self, id: PickerId) {
        match id {
            PickerId::SubsonicSearch => {
                self.refresh_subsonic_status();
                self.subsonic.search_results = SubsonicSearchResults::default();
            }
            PickerId::SubsonicAlbums => {
                self.subsonic.albums.clear();
                self.subsonic.albums_pending = true;
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.subsonic().albums(0, 200).await {
                        Ok(a) => {
                            let _ = ipc_tx.send(IpcResult::SubsonicAlbums(a));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("subsonic albums failed: {e}"));
                        }
                    }
                });
            }
            PickerId::SubsonicAlbumTracks => {
                if let Some(album) = self.subsonic.selected_album.clone() {
                    let album_id = album.id;
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    let title = album.title;
                    tokio::spawn(async move {
                        match c.subsonic().album_tracks(&album_id).await {
                            Ok(t) => {
                                let _ = ipc_tx.send(IpcResult::SubsonicAlbumTracks(t));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("subsonic album '{title}' failed: {e}"));
                            }
                        }
                    });
                }
            }
            PickerId::SubsonicSetup => {
                if let Some(st) = self.subsonic.status.clone().filter(|st| st.configured) {
                    self.subsonic.form_server = st.server.unwrap_or_default();
                    self.subsonic.form_user = st.user.unwrap_or_default();
                }
                self.refresh_subsonic_status();
            }
            PickerId::PodcastFeeds => {
                self.podcast.feeds_pending = true;
                self.refresh_subsonic_status();
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.podcast().feeds().await {
                        Ok(f) => {
                            let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                            match c.podcast().status().await {
                                Ok(s) => {
                                    let _ = ipc_tx.send(IpcResult::PodcastStatus(Some(s)));
                                }
                                Err(e) => {
                                    self_err(&ipc_tx, format!("podcast status failed: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                        }
                    }
                });
            }
            PickerId::PodcastEpisodes => {
                if let Some(feed_id) = self.podcast.episodes_feed_id.clone() {
                    self.fetch_podcast_episodes(feed_id);
                }
            }
            PickerId::RadioTop => {
                self.radio.top_pending = true;
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.radio().top(50).await {
                        Ok(s) => {
                            let _ = ipc_tx.send(IpcResult::RadioTop(s));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("radio top failed: {e}"));
                        }
                    }
                });
            }
            PickerId::RadioBrowse => {}
            PickerId::RadioBrowseList => {
                self.fetch_radio_browse_list();
            }
            PickerId::RadioBrowseStations => {
                self.fetch_radio_browse_stations();
            }
            PickerId::Setup => {
                self.refresh_subsonic_status();
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.lastfm().status().await {
                        Ok(st) => {
                            let _ = ipc_tx.send(IpcResult::LastfmStatus(Some(st)));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("last.fm status failed: {e}"));
                        }
                    }
                    match c.spotify().status().await {
                        Ok(st) => {
                            let _ = ipc_tx.send(IpcResult::SpotifyStatus(st));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("spotify status failed: {e}"));
                        }
                    }
                });
            }
            PickerId::LastfmAuth => self.refresh_lastfm_status(),
            _ => {}
        }
    }

    /// Re-pull the Last.fm link status into the setup view.
    pub fn refresh_lastfm_status(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.lastfm().status().await {
                Ok(st) => {
                    let _ = ipc_tx.send(IpcResult::LastfmStatus(Some(st)));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("last.fm status failed: {e}"));
                }
            }
        });
    }

    pub fn refresh_subsonic_status(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.subsonic().status().await {
                Ok(st) => {
                    let _ = ipc_tx.send(IpcResult::SubsonicStatus(Some(st)));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("subsonic status failed: {e}"));
                }
            }
        });
    }

    pub fn subsonic_search(&mut self, query: String) {
        self.subsonic.search_results = SubsonicSearchResults::default();
        self.subsonic.search_pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.subsonic().search(&query).await {
                Ok(res) => {
                    let _ = ipc_tx.send(IpcResult::SubsonicSearch(res));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("subsonic search failed: {e}"));
                }
            }
        });
    }

    pub fn fetch_subsonic_cover(&mut self, track_id: String) {
        if self.subsonic.cover_track_id.as_deref() == Some(track_id.as_str()) {
            return;
        }
        self.subsonic.cover_track_id = Some(track_id.clone());
        self.subsonic.cover_preview = None;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.subsonic().cover(&track_id).await {
                Ok(data) => {
                    let _ = ipc_tx.send(IpcResult::SubsonicCover(data));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("subsonic cover failed: {e}"));
                }
            }
        });
    }

    pub fn fetch_podcast_episodes(&mut self, feed_id: String) {
        self.podcast.episodes.clear();
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let fid = feed_id.clone();
        tokio::spawn(async move {
            match c.podcast().episodes(&fid).await {
                Ok((_title, eps)) => {
                    let _ = ipc_tx.send(IpcResult::PodcastEpisodes(eps));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("podcast episodes failed: {e}"));
                }
            }
        });
        self.podcast.episodes_feed_id = Some(feed_id);
    }

    pub fn search_radio(&mut self, query: String) {
        self.radio.search.clear();
        self.radio.search_pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.radio().search(&query, 50).await {
                Ok(s) => {
                    let _ = ipc_tx.send(IpcResult::RadioSearch(s));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio search failed: {e}"));
                }
            }
        });
    }

    /// (Re)load the tag or country list for the RadioBrowseList picker.
    pub fn fetch_radio_browse_list(&mut self) {
        self.radio.browse_pending = true;
        let (c, ipc_tx) = (self.client.clone(), self.ipc_tx.clone());
        let kind = self.radio.browse_kind;
        tokio::spawn(async move {
            let r = match kind {
                RadioBrowseKind::Tags => c.radio().tags(200).await.map(IpcResult::RadioTags),
                RadioBrowseKind::Countries => c
                    .radio()
                    .countries(200)
                    .await
                    .map(IpcResult::RadioCountries),
            };
            match r {
                Ok(ipc) => {
                    let _ = ipc_tx.send(ipc);
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio browse failed: {e}"));
                }
            }
        });
    }

    /// (Re)load the stations for the tag/country selected at RadioBrowseList.
    pub fn fetch_radio_browse_stations(&mut self) {
        self.radio.browse_stations.clear();
        self.radio.browse_stations_pending = true;
        let (c, ipc_tx) = (self.client.clone(), self.ipc_tx.clone());
        let topic = self.radio.browse_topic.clone();
        let kind = self.radio.browse_kind;
        tokio::spawn(async move {
            let r = match kind {
                RadioBrowseKind::Tags => c
                    .radio()
                    .stations_by_tag(&topic, 50)
                    .await
                    .map(IpcResult::RadioBrowseStations),
                RadioBrowseKind::Countries => c
                    .radio()
                    .stations_by_country(&topic, 50)
                    .await
                    .map(IpcResult::RadioBrowseStations),
            };
            match r {
                Ok(ipc) => {
                    let _ = ipc_tx.send(ipc);
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio stations failed: {e}"));
                }
            }
        });
    }

    /// Filtered tracks for the current library view, respecting search query, browse_detail, and category.
    /// Selection index for the currently active library list (per-category,
    /// see the `scroll_offset` field).
    pub fn list_pos(&self) -> usize {
        let i = self.library_category.min(LIBRARY_CATEGORIES.len() - 1);
        self.scroll_offset[i]
    }

    /// Set the selection index for the currently active library list.
    pub fn set_list_pos(&mut self, v: usize) {
        let i = self.library_category.min(LIBRARY_CATEGORIES.len() - 1);
        self.scroll_offset[i] = v;
    }

    /// Fully reset the library-left-pane view to a target category + drill-down.
    /// Clears the list position, multiselect selection and drill-down caches so
    /// a stale Enter/toggle can never act on a row from a previous view.
    fn reset_library_view(&mut self, category: usize, detail: Option<String>) {
        self.browse_detail = detail;
        self.library_category = category.min(LIBRARY_CATEGORIES.len() - 1);
        self.selected_indices.clear();
        self.playlist_tracks_cache.clear();
        self.spotify.playlist_tracks_cache.clear();
        self.set_list_pos(0);
    }

    pub fn filtered_tracks(&self) -> Vec<&TrackInfo> {
        if self.library_category == 4 && self.browse_detail.is_some() {
            return self.playlist_tracks_cache.iter().collect();
        }
        let mut tracks: Vec<&TrackInfo> = self.tracks_cache.iter().collect();
        if !self.search_query.is_empty() {
            let q = self.search_query.to_lowercase();
            tracks.retain(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            });
        }
        if let Some(ref detail) = self.browse_detail {
            // Drill-down must match the browse key exactly: a substring OR
            // across album/artist/title pulls in unrelated tracks (e.g. an
            // album name that appears in another track's title) and misses
            // empty-field keys that `unique_albums`/`unique_artists` render
            // as "Unknown Album"/"Unknown Artist".
            tracks.retain(|t| match self.library_category {
                2 => {
                    let album: &str = if t.album.is_empty() {
                        "Unknown Album"
                    } else {
                        &t.album
                    };
                    album.eq_ignore_ascii_case(detail)
                }
                3 => {
                    let artist: &str = if t.artist.is_empty() {
                        "Unknown Artist"
                    } else {
                        &t.artist
                    };
                    artist.eq_ignore_ascii_case(detail)
                }
                _ => {
                    t.album.eq_ignore_ascii_case(detail)
                        || t.artist.eq_ignore_ascii_case(detail)
                        || t.title.eq_ignore_ascii_case(detail)
                }
            });
        }
        if self.library_category == 1 {
            tracks.retain(|t| t.favourite);
        } else if self.library_category == 5 {
            // Spotify: category renders the synced playlist browser, not a flat
            // TrackInfo list: resolve/play goes through the daemon.
            tracks.clear();
        }
        // Sorting applies to the flat track list (All Tracks / Favourites and the
        // album/artist drill-downs). Playlist and Spotify views sort upstream.
        if self.browse_detail.is_none() && self.library_category <= 1 {
            match self.track_sort {
                TrackSort::Recents => {
                    tracks.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.title.cmp(&b.title)));
                }
                TrackSort::RecentlyAdded => {
                    tracks.sort_by_key(|a| std::cmp::Reverse(a.id));
                }
                TrackSort::Alphabetical => {
                    tracks.sort_by(|a, b| {
                        a.title
                            .to_lowercase()
                            .cmp(&b.title.to_lowercase())
                            .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
                    });
                }
                TrackSort::Artist => {
                    tracks.sort_by(|a, b| {
                        a.artist
                            .to_lowercase()
                            .cmp(&b.artist.to_lowercase())
                            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                    });
                }
                TrackSort::Album => {
                    tracks.sort_by(|a, b| {
                        a.album
                            .to_lowercase()
                            .cmp(&b.album.to_lowercase())
                            .then_with(|| {
                                a.track_number
                                    .cmp(&b.track_number)
                                    .then_with(|| a.title.cmp(&b.title))
                            })
                    });
                }
            }
        }
        tracks
    }

    /// Play the track highlighted in the current library view, replacing the
    /// queue with the filtered list and starting at that row.
    fn play_filtered_highlighted(&self) {
        let filtered = self.filtered_tracks();
        let idx = self.list_pos();
        if idx >= filtered.len() {
            return;
        }
        let paths: Vec<String> = filtered.iter().map(|t| t.path.clone()).collect();
        let path = paths[idx].clone();
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.queue().set(paths, idx as u64).await;
            let _ = c.play(&path, 0.0).await;
        });
    }

    /// Unique album names with track counts, sorted by album.
    pub fn unique_albums(&self) -> Vec<(String, usize)> {
        let mut albums: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.album.is_empty() {
                "Unknown Album".into()
            } else {
                t.album.clone()
            };
            *albums.entry(key).or_insert(0) += 1;
        }
        albums.into_iter().collect()
    }

    /// Length of the list currently visible in the library right pane,
    /// depending on the active category and drill-down state.
    pub fn library_list_len(&self) -> usize {
        if self.browse_detail.is_some() {
            if self.library_category == 5 {
                return self.spotify_playlist_rows();
            }
            return self.filtered_tracks().len();
        }
        match self.library_category {
            2 => self.unique_albums().len(),
            3 => self.unique_artists().len(),
            4 => self.playlist_cache.len(),
            5 => self.spotify.playlists.len(),
            _ => self.filtered_tracks().len(),
        }
    }

    /// Virtual action rows (Play All / Shuffle) prepended to a Spotify playlist
    /// drill-down track list.
    pub const SPOTIFY_PLAYLIST_ACTION_ROWS: usize = 2;

    /// True while the right pane is showing the track list of a Spotify
    /// playlist (drilled down from the Spotify playlists category).
    pub fn in_spotify_playlist(&self) -> bool {
        self.browse_detail.is_some() && self.library_category == 5
    }

    /// Row count of the Spotify playlist drill-down list, including the two
    /// virtual action rows (`Play All`, `Shuffle`) at the top.
    pub fn spotify_playlist_rows(&self) -> usize {
        self.spotify.playlist_tracks_cache.len() + Self::SPOTIFY_PLAYLIST_ACTION_ROWS
    }

    /// The track the current list position maps to in a Spotify playlist
    /// drill-down. `None` for the action rows and non-Spotify views.
    pub fn selected_spotify_track(&self) -> Option<&SpotifyTrack> {
        if !self.in_spotify_playlist() {
            return None;
        }
        self.spotify.playlist_tracks_cache.get(
            self.list_pos()
                .saturating_sub(Self::SPOTIFY_PLAYLIST_ACTION_ROWS),
        )
    }

    /// Track ids owned by the list position `pos` (when that row maps to a
    /// concrete track). Returns `None` for album/artist/playlist/spotify rows
    /// whose cover is derived from a different key.
    fn track_id_at(&self, pos: usize) -> Option<i64> {
        if self.browse_detail.is_some() {
            // Spotify drill-down rows are remote tracks (no local id to warm
            // the disk-cache with); only local track rows have an id to preload.
            if self.library_category != 5 {
                return self.filtered_tracks().get(pos).map(|t| t.id);
            }
            return None;
        }
        match self.library_category {
            0 | 1 => self.filtered_tracks().get(pos).map(|t| t.id),
            _ => None,
        }
    }

    /// Preload the cover art for the tracks a short scroll ahead of the cursor
    /// so fast scrolling (e.g. holding an arrow key) warms the daemon's
    /// disk/LRU cache and the on-selection fetch becomes a cache hit. Fires in
    /// the background and never blocks the UI or surfaces errors.
    pub fn preload_upcoming_covers(&mut self) {
        let pos = self.list_pos();
        let mut ids = Vec::new();
        for off in 1..=3 {
            if let Some(id) = self.track_id_at(pos + off) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            for id in ids {
                // Errors (track without cover / daemon lookup fail) are fine:
                // a warm miss is simply skipped next time.
                let _ = client.art().cover(id).await;
            }
        });
    }

    /// Unique artist names with track counts, sorted by artist.
    pub fn unique_artists(&self) -> Vec<(String, usize)> {
        let mut artists: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.artist.is_empty() {
                "Unknown Artist".into()
            } else {
                t.artist.clone()
            };
            *artists.entry(key).or_insert(0) += 1;
        }
        artists.into_iter().collect()
    }

    fn cover_sync(&mut self) {
        match (&self.np_cover.image, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.np_cover.stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.np_cover.stateful = None;
                }
            }
            _ => self.np_cover.stateful = None,
        }
    }

    fn popup_cover_sync(&mut self) {
        match (&self.track_popup_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.popup_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.popup_cover_stateful = None;
                }
            }
            _ => self.popup_cover_stateful = None,
        }
    }

    fn upnext_cover_sync(&mut self) {
        let Some(picker) = self.np_cover.picker.as_ref() else {
            return;
        };
        if let Some(u) = self.upnext.as_mut() {
            u.cover_stateful = match u.cover.as_ref() {
                Some(bytes) => image::load_from_memory(bytes)
                    .ok()
                    .map(|img| picker.new_resize_protocol(img)),
                None => None,
            };
        }
    }

    /// Fetch cover art for the queue picker "Up Next" strip, once per
    /// track.  Locally-inserted tracks (`id == 0`) are fetched too; if the
    /// daemon has no art the renderer falls back to a glyph.
    pub fn update_queue_preview_cover(&mut self) {
        if no_image_protocol() {
            return;
        }
        let next_idx = self.queue.cursor + 1;
        let Some(track) = self.queue.cache.get(next_idx) else {
            self.queue.last_preview_cover_fetch_id = None;
            self.queue.last_preview_cover_fetch_gen = None;
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
            return;
        };
        let tid = track.id;
        let track_path = track.path.clone();
        // Any cached cover bytes must belong to the track currently shown as
        // up-next. A cursor jump, queue replacement, or thumbnail clear can
        // reset the fetch guard without invalidating the bytes; dropping them
        // here guarantees the preview can never show art for the previous
        // track (rendered from a stale `cover_block` fallback).
        if self.queue.preview_cover.is_some() && self.queue.last_preview_cover_fetch_id != Some(tid)
        {
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
        }
        // A failed lookup clears the gen guard so a later preview can retry;
        // this throttle prevents the per-frame render from re-fetching a
        // cover that isn't there, at most once per 30s per track.
        if let Some((fail_tid, until)) = self.queue.preview_cover_fail_until
            && fail_tid == tid
            && std::time::Instant::now() < until
        {
            return;
        }
        // If the up-next notification refers to the very same track that the
        // queue picker is showing, reuse its cover bytes so both surfaces are
        // always in sync and the now-playing cover can never appear here.
        let reuse = self
            .upnext
            .as_ref()
            .filter(|u| {
                u.track.id == tid
                    && u.track.path == track_path
                    && u.cover.is_some()
                    && self.queue.last_preview_cover_fetch_id != Some(tid)
            })
            .and_then(|u| u.cover.clone());
        if let Some(cover) = reuse {
            self.queue.preview_cover = Some(cover);
            self.queue_preview_cover_sync();
            self.queue.last_preview_cover_fetch_id = Some(tid);
            self.queue.last_preview_cover_fetch_gen = None;
            return;
        }
        // Generation-guarded dedup: allows `id == 0` tracks to refetch
        // distinctly. Only skip when pending fetch_gen exists.
        if self.queue.last_preview_cover_fetch_id == Some(tid)
            && self.queue.last_preview_cover_fetch_gen.is_some()
        {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.queue.last_preview_cover_fetch_id = Some(tid);
        self.queue.last_preview_cover_fetch_gen = Some(fetch_gen);
        self.queue.preview_cover = None;
        self.queue.preview_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // Report failure as `None` too, so the pending-gen guard is
            // released and a later save/queue change can retry the lookup.
            let cover = if let Ok(Some(b64)) = client.art().cover_for(tid, Some(track_path)).await {
                base64::engine::general_purpose::STANDARD.decode(&b64).ok()
            } else {
                None
            };
            let _ = ipc_tx.send(IpcResult::QueuePreviewCover(cover, tid, fetch_gen));
        });
    }

    fn queue_preview_cover_sync(&mut self) {
        match (&self.queue.preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.queue.preview_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.queue.preview_cover_stateful = None;
                }
            }
            _ => self.queue.preview_cover_stateful = None,
        }
    }

    fn picker_preview_sync(&mut self) {
        match (&self.picker_preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.picker_preview_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.picker_preview_stateful = None;
                }
            }
            _ => self.picker_preview_stateful = None,
        }
    }

    /// (Re)build the stateful cover protocol for the Edit Metadata preview.
    fn metadata_cover_sync(&mut self) {
        match (&self.metadata.cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.metadata.cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.metadata.cover_stateful = None;
                }
            }
            _ => self.metadata.cover_stateful = None,
        }
    }

    fn artist_cover_sync(&mut self) {
        match (&self.artist_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.artist_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.artist_cover_stateful = None;
                }
            }
            _ => self.artist_cover_stateful = None,
        }
    }

    fn spotify_preview_sync(&mut self) {
        match (&self.spotify.preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.spotify.preview_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.spotify.preview_cover_stateful = None;
                }
            }
            _ => self.spotify.preview_cover_stateful = None,
        }
    }

    /// Fetch cover art for the highlighted SpotifySearch picker row (a web
    /// search hit carrying an album-cover URL) so the preview window can render
    /// it as ASCII, mirroring the SearchLibrary picker behaviour.
    pub fn update_spotify_search_preview(&mut self) {
        let Some(top) = self.pickers.top() else {
            self.spotify.preview_cover = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.last_preview_fetch = None;
            self.spotify.last_preview_fetch_gen = None;
            return;
        };
        if top.id != PickerId::SpotifySearch {
            self.spotify.preview_cover = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.last_preview_fetch = None;
            self.spotify.last_preview_fetch_gen = None;
            return;
        }
        if self.spotify.search_results.is_empty() {
            self.spotify.preview_cover = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.last_preview_fetch = None;
            self.spotify.last_preview_fetch_gen = None;
            return;
        }
        let sel = top
            .selected
            .min(self.spotify.search_results.len().saturating_sub(1));
        let Some(url) = self.spotify.search_results[sel].2.image_url.clone() else {
            self.spotify.preview_cover = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.last_preview_fetch = None;
            self.spotify.last_preview_fetch_gen = None;
            return;
        };
        if self.spotify.last_preview_fetch.as_deref() == Some(&url)
            && self.spotify.last_preview_fetch_gen.is_some()
        {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.spotify.last_preview_fetch = Some(url.clone());
        self.spotify.last_preview_fetch_gen = Some(fetch_gen);
        self.spotify.preview_cover = None;
        self.spotify.preview_cover_stateful = None;
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match client.spotify().track_image(&url).await {
                Ok(Some(b64)) => {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                        let _ = ipc_tx.send(IpcResult::SpotifyPreviewCover(
                            Some(bytes),
                            url,
                            fetch_gen,
                        ));
                    }
                }
                Ok(None) | Err(_) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyPreviewCover(None, url, fetch_gen));
                }
            }
        });
    }

    /// Fetch the cover art for the track currently being edited and stream it
    /// to the `MetadataCoverArt` IPC channel so the preview can refresh.
    /// Generation-guarded to prevent stale picker-reuse overwrites.
    fn fetch_metadata_cover(&mut self) {
        let Some(track_id) = self.metadata.edit_track_id else {
            return;
        };
        if no_image_protocol() {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.metadata.cover_fetch_gen = Some(fetch_gen);
        // Clear stale cover while new fetch is in flight; handler will repopulate.
        self.metadata.cover = None;
        self.metadata.cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(track_id).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::MetadataCoverArt(
                    Some(bytes),
                    track_id,
                    fetch_gen,
                ));
            }
        });
    }

    fn settings_options_for_category(&self) -> usize {
        match self.settings_category {
            0 => 4,  // YouTube: Cookie Source, Cookie File, JS Runtime, Auto Download
            1 => 6,  // Playback: Repeat, Shuffle, Crossfade, EQ Enabled, Reverb, Cover Source
            2 => 14, // System: Theme, Transparent BG, Transparent Pickers, Sync Covers, Sync Lyrics, Sync Metadata, Footer Preset, Visualizer, Reactive Theme, Reactive Intensity, Hide Footer, Clear Lyrics Cache, Clear Cover Cache, Notification Settings, Theme Mode
            3 => 7,  // Spotify: Status, Account, Playlists, Link, Sync, Unlink, Device
            _ => 0,
        }
    }

    /// Cycle the visibility mode of the notification category selected in the
    /// `NotificationSettings` picker. +1 advances Floating → Footer → Off, -1
    /// reverses. Changes persist immediately through the normal prefs path.
    fn cycle_notification_mode(&mut self, dir: isize) {
        let Some(top) = self.pickers.top() else {
            return;
        };
        let idx = top.selected.min(NotifType::ALL.len() - 1);
        let ntype = NotifType::ALL[idx];
        let pos = NotifMode::ALL
            .iter()
            .position(|m| {
                self.notification_modes
                    .get(&ntype)
                    .copied()
                    .unwrap_or(NotifMode::Floating)
                    == *m
            })
            .unwrap_or(0) as isize;
        let next = NotifMode::ALL
            .get(((pos + dir).rem_euclid(NotifMode::ALL.len() as isize)) as usize)
            .copied()
            .unwrap_or(NotifMode::Floating);
        self.notification_modes.insert(ntype, next);
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "System",
            format!("{} notifications: {}", ntype.label(), next.label()),
            NotificationKind::Info,
            true,
            NotifType::System,
        );
    }

    async fn fetch_queue(&mut self) {
        if let Ok(DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        }) = self.client.queue().list().await
        {
            self.queue.cache = tracks;
            self.queue.cursor = cursor as usize;
        }
    }

    fn handle_command(&mut self, cmd: TuiCommand) {
        // All IPC calls are spawned as background tasks to avoid blocking the UI loop.
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let err_tx = ipc_tx.clone();
        let error_handler = move |e: CoreError| {
            let _ = err_tx.send(IpcResult::Error(e.to_string()));
        };
        let err_tx2 = ipc_tx.clone();
        let error_handler2 = move |e: CoreError| {
            let _ = err_tx2.send(IpcResult::Error(e.to_string()));
        };

        match cmd {
            TuiCommand::Play(path) => {
                tokio::spawn(async move {
                    if let Err(e) = client.play(&path, 0.0).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::PlayPause => {
                tokio::spawn(async move {
                    if let Err(e) = client.play_pause().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Pause => {
                tokio::spawn(async move {
                    if let Err(e) = client.pause().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Stop => {
                tokio::spawn(async move {
                    if let Err(e) = client.stop().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Next => {
                self.manual_track_advance = true;
                tokio::spawn(async move {
                    if let Err(e) = client.next().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Prev => {
                self.manual_track_advance = true;
                tokio::spawn(async move {
                    if let Err(e) = client.prev().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Seek(pos) => {
                // Mirror the seek into the local position estimate immediately so
                // the lyric highlight follows the new position right away (not a
                // full daemon round-trip later), and clear the monotonic guard so a
                // backward seek isn't clamped. `estimated_position` also honours
                // this seek target while it is fresh.
                self.seek_pending = Some(std::time::Instant::now());
                if self.state.current_track.is_some() {
                    self.raw_position = pos;
                    self.display_position = pos;
                }
                tokio::spawn(async move {
                    if let Err(e) = client.seek(pos).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetVolume(v) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_volume(v).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetSpeed(r) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_speed(r).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetLowPower(enabled) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_low_power(enabled).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::ToggleShuffle => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_shuffle().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::CycleRepeat(m) => {
                tokio::spawn(async move {
                    if let Err(e) = client.cycle_repeat(m).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::ToggleMute => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_mute().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Crossfade(en, dur) => {
                tokio::spawn(async move {
                    if let Err(e) = client.crossfade(en, dur).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::QueueAdd(p) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().add(&p, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::QueueMove(from, to) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().reorder(from, to).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::QueueClear => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().clear().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::YtSearch(q) => {
                self.yt_search_loading = true;
                self.yt_results_cache.clear();
                let c = client.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.yt().search(&q, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::YtDownload { url, title, artist } => {
                self.notify_titled(
                    "YouTube",
                    "Download started…",
                    NotificationKind::Info,
                    false,
                    NotifType::Downloads,
                );
                let ipc = ipc_tx.clone();
                let client2 = self.client.clone();
                tokio::spawn(async move {
                    let msg = async {
                        // Kick off the daemon-side yt-dlp download (fresh PO
                        // token + signature extraction on every run).
                        let _ = match client2
                            .yt()
                            .download(url.clone(), title.clone(), artist.clone())
                            .await
                        {
                            Ok(id) => id,
                            Err(e) => return format!("Download error: {e}"),
                        };
                        let deadline = std::time::Instant::now() + Duration::from_secs(600);
                        let file_path = loop {
                            if std::time::Instant::now() > deadline {
                                let _ = client2.yt().cancel_download(url.clone()).await;
                                return "Download timed out".to_string();
                            }
                            let res = match client2.yt().download_poll().await {
                                Ok(res) => res,
                                Err(e) => return format!("Download error: {e}"),
                            };
                            match &res {
                                DaemonRes::YtDownloadProgress {
                                    id,
                                    url,
                                    title,
                                    progress,
                                    status,
                                    error,
                                    file_path: fp,
                                    downloaded_bytes,
                                    total_bytes,
                                    rate_bytes_per_sec,
                                    eta_secs,
                                } => {
                                    // Mirror live progress to the TUI footer so
                                    // the user sees the download moving.
                                    let _ = ipc.send(IpcResult::YtDownloadProgress {
                                        id: *id,
                                        url: url.clone(),
                                        title: title.clone(),
                                        progress: *progress,
                                        status: status.clone(),
                                        file_path: fp.clone(),
                                        downloaded_bytes: *downloaded_bytes,
                                        total_bytes: *total_bytes,
                                        rate_bytes_per_sec: *rate_bytes_per_sec,
                                        eta_secs: *eta_secs,
                                    });
                                    if let Some(fp) = fp.clone().filter(|_| status == "completed") {
                                        break fp.clone();
                                    }
                                    if status == "failed" || status == "cancelled" {
                                        return error
                                            .clone()
                                            .unwrap_or_else(|| format!("Download {status}"));
                                    }
                                }
                                DaemonRes::Error { message } => {
                                    return format!("Download failed: {message}");
                                }
                                DaemonRes::Value { .. } => {}
                                _ => {
                                    return "Download error: unexpected daemon response"
                                        .to_string();
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        };
                        let audio_dir = std::path::PathBuf::from(&file_path)
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| std::path::PathBuf::from("."));

                        let _ = client2
                            .library()
                            .scan(audio_dir.to_string_lossy().as_ref())
                            .await;
                        // Also refresh the track cache
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client2.library().get_tracks(None, None).await
                        {
                            let _ = ipc.send(IpcResult::LibraryTracks(tracks));
                        }
                        // Try to fetch lyrics for the newly downloaded track
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client2.library().get_tracks(None, None).await
                            && let Some(track) = tracks
                                .iter()
                                .find(|t| t.path.contains(&file_path))
                                .or_else(|| tracks.last())
                        {
                            if let Ok(Some(lyrics_data)) =
                                client2.lyrics().get(track.id, Some(&track.path)).await
                                && !lyrics_data.lines.is_empty()
                            {
                                // Write .lrc sidecar next to the audio file
                                let lrc_path = {
                                    let p = std::path::PathBuf::from(&track.path);
                                    p.with_extension("lrc")
                                };
                                let mut lrc_content = String::new();
                                if let Some(ref ar) = lyrics_data.artist {
                                    lrc_content.push_str(&format!("[ar:{}]\n", ar));
                                }
                                if let Some(ref al) = lyrics_data.album {
                                    lrc_content.push_str(&format!("[al:{}]\n", al));
                                }
                                if let Some(ref ti) = lyrics_data.title {
                                    lrc_content.push_str(&format!("[ti:{}]\n", ti));
                                }
                                for line in &lyrics_data.lines {
                                    if line.timestamp < 0.0 {
                                        lrc_content.push_str(&line.text);
                                        lrc_content.push('\n');
                                        continue;
                                    }
                                    let mins = (line.timestamp / 60.0) as u64;
                                    let secs = line.timestamp - (mins as f64 * 60.0);
                                    lrc_content.push_str(&format!(
                                        "[{:02}:{:05.2}]{}\n",
                                        mins, secs, line.text
                                    ));
                                }
                                let _ = std::fs::write(&lrc_path, lrc_content);
                            }
                            // Scrub the downloaded file's tags and fetch
                            // its cover art (runs per-path in the
                            // background).
                            let _ = client2
                                .library()
                                .sync_metadata(Some(track.path.clone()))
                                .await;
                        }
                        match (&title, &artist) {
                            (Some(t), Some(a)) => format!("Downloaded: {} - {}", a, t),
                            (Some(t), _) => format!("Downloaded: {}", t),
                            _ => {
                                let name = std::path::Path::new(&file_path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| file_path.clone());
                                format!("Downloaded: {}", name)
                            }
                        }
                    }
                    .await;
                    let kind = if msg.starts_with("Downloaded") {
                        NotificationKind::Success
                    } else {
                        NotificationKind::Error
                    };
                    let _ = ipc.send(IpcResult::Notification(
                        "YouTube".to_string(),
                        msg,
                        kind,
                        NotifType::Downloads,
                    ));
                });
            }
            TuiCommand::YtResolve(u) => {
                tokio::spawn(async move {
                    if let Err(e) = client.yt().resolve_stream(&u).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::SetEqPreset(preset) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_eq_preset(preset).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Search(q) => {
                tokio::spawn(async move {
                    if let Err(e) = client.search(&q).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::AddFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.favourites().add(id).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::RemoveFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.favourites().remove(id).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Refresh => {
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = client2.get_status().await {
                        // Cover art is fetched on track-change events, not
                        // here, to avoid an extra IPC call every second.
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                    // Also refresh queue to recover from initial background
                    // spawn failures that leave queue_cache empty.
                    if let Ok(DaemonRes::QueueState {
                        queue: tracks,
                        cursor,
                        ..
                    }) = client2.queue().list().await
                    {
                        let _ = ipc_tx2.send(IpcResult::Queue(tracks, cursor as usize));
                    }
                });
            }
            TuiCommand::RefreshLibrary => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::Tracks { tracks, .. }) =
                        client.library().get_tracks(None, None).await
                    {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                });
            }
            TuiCommand::RefreshYt => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::YtSearchResults { query, results }) =
                        client.yt().poll().await
                    {
                        let _ = ipc_tx.send(IpcResult::YtResults(query, results));
                    }
                });
            }
            TuiCommand::RemoveTrack(track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.library().remove_track(track_id).await {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Library".to_string(),
                            "Track deleted".to_string(),
                            NotificationKind::Success,
                            NotifType::Library,
                        ));
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client.library().get_tracks(None, None).await
                        {
                            let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                        }
                    }
                });
            }
            TuiCommand::RemoveFromPlaylist(playlist_id, track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client
                        .library()
                        .remove_from_playlist(playlist_id, track_id)
                        .await
                    {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Playlist".to_string(),
                            "Removed from playlist".to_string(),
                            NotificationKind::Success,
                            NotifType::NowPlaying,
                        ));
                        if let Ok(DaemonRes::Playlists { playlists, .. }) =
                            client.library().get_playlists().await
                        {
                            let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                        }
                    }
                });
            }
            TuiCommand::FetchLyrics => {
                let track_path = self.state.current_track.as_ref().map(|t| t.path.clone());
                let track_id = self.state.current_track.as_ref().map(|t| t.id).unwrap_or(0);
                let fetch_gen = self.next_lyrics_gen();
                self.lyrics.pending_gen = Some(fetch_gen);
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(5),
                        client2.lyrics().get(track_id, track_path.as_deref()),
                    )
                    .await;
                    match result {
                        Ok(Ok(lyrics)) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(lyrics, fetch_gen));
                        }
                        Ok(Err(e)) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(None, fetch_gen));
                            let _ = ipc_tx2.send(IpcResult::Error(format!("Lyrics: {e}")));
                        }
                        Err(_) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(None, fetch_gen));
                            let _ = ipc_tx2
                                .send(IpcResult::Error("Lyrics fetch timed out".to_string()));
                        }
                    }
                });
            }
            TuiCommand::SetSleepTimer(minutes) => {
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.set_sleep_timer(minutes).await {
                        let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    }
                });
            }
            TuiCommand::CancelSleepTimer => {
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.cancel_sleep_timer().await {
                        let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    }
                });
            }
            TuiCommand::CheckHealth => {
                self.show_health_on_report = true;
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match client.check_health().await {
                        Ok(report) => {
                            let _ = ipc_tx.send(IpcResult::HealthReport(report));
                        }
                        Err(e) => {
                            let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                        }
                    }
                });
            }
        };
    }

    fn set_last_action(&mut self, name: &str) {
        self.last_action_name = Some((
            name.to_string(),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    fn clamp_picker_selection(&mut self) {
        let (id, query) = match self.pickers.top_mut() {
            Some(t) => (t.id, t.query.clone()),
            None => return,
        };
        let max = match id {
            PickerId::Queue => self.queue.cache.len().saturating_sub(1),
            PickerId::YTSearch => self.yt_results_cache.len().saturating_sub(1),
            PickerId::SearchLibrary => self.search_library_picks().len().saturating_sub(1),
            PickerId::Equalizer => EQ_PRESETS.len().saturating_sub(1),
            PickerId::SleepTimer => 6,
            PickerId::Crossfade => 13,
            PickerId::VisualizerPreset => VisualizerPreset::all().len().saturating_sub(1),
            PickerId::FooterPreset => self.footer_presets.len().saturating_sub(1),
            PickerId::ProgressStyle => ProgressStyle::all().len().saturating_sub(1),
            PickerId::Notifications => self.notification_history.len().saturating_sub(1),
            PickerId::NotificationSettings => NotifType::ALL.len().saturating_sub(1),
            PickerId::PlaylistSelect => self.playlist_cache.len(),
            PickerId::PlaylistTrackSelect => self.tracks_cache.len().saturating_sub(1),
            PickerId::ThemePicker => {
                let q = query.to_lowercase();
                if q.is_empty() {
                    self.themes.len()
                } else {
                    self.themes
                        .iter()
                        .filter(|entry| {
                            let lower = entry.name.to_lowercase();
                            let mut qi = 0usize;
                            for ch in lower.chars() {
                                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                    qi += 1;
                                }
                            }
                            qi == q.len()
                        })
                        .count()
                }
                .saturating_sub(1)
            }
            PickerId::CommandPalette => {
                let commands = CommandPalette::commands(&self.icon_style);
                let q = query.to_lowercase();
                if q.is_empty() {
                    commands.len()
                } else {
                    commands
                        .iter()
                        .filter(|c| {
                            let lower = c.icon.to_lowercase();
                            let mut qi = 0usize;
                            for ch in lower.chars() {
                                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                    qi += 1;
                                }
                            }
                            qi == q.len()
                        })
                        .count()
                }
                .saturating_sub(1)
            }
            _ => usize::MAX,
        };
        if let Some(top) = self.pickers.top_mut() {
            top.selected = top.selected.min(max);
        }
    }

    /// Returns the item count (max+1) for wrap navigation.
    fn picker_item_count(&self) -> usize {
        let (id, query) = match self.pickers.top() {
            Some(t) => (t.id, t.query.clone()),
            None => return 0,
        };
        match id {
            PickerId::Queue => self.queue.cache.len(),
            PickerId::YTSearch => self.yt_results_cache.len(),
            PickerId::SearchLibrary => self.search_library_picks().len(),
            PickerId::Equalizer => EQ_PRESETS.len(),
            PickerId::SleepTimer => 7,
            PickerId::Crossfade => 14,
            PickerId::VisualizerPreset => VisualizerPreset::all().len(),
            PickerId::FooterPreset => self.footer_presets.len(),
            PickerId::ProgressStyle => ProgressStyle::all().len(),
            PickerId::Notifications => self.notification_history.len(),
            PickerId::NotificationSettings => NotifType::ALL.len(),
            PickerId::PlaylistSelect => self.playlist_cache.len() + 1,
            PickerId::PlaylistTrackSelect => self.tracks_cache.len(),
            PickerId::ThemePicker => {
                let q = query.to_lowercase();
                if q.is_empty() {
                    self.themes.len()
                } else {
                    self.themes
                        .iter()
                        .filter(|entry| {
                            let lower = entry.name.to_lowercase();
                            let mut qi = 0usize;
                            for ch in lower.chars() {
                                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                    qi += 1;
                                }
                            }
                            qi == q.len()
                        })
                        .count()
                }
            }
            PickerId::CommandPalette => {
                let commands = CommandPalette::commands(&self.icon_style);
                let q = query.to_lowercase();
                if q.is_empty() {
                    commands.len()
                } else {
                    commands
                        .iter()
                        .filter(|c| {
                            let lower = c.icon.to_lowercase();
                            let mut qi = 0usize;
                            for ch in lower.chars() {
                                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                    qi += 1;
                                }
                            }
                            qi == q.len()
                        })
                        .count()
                }
            }
            PickerId::SubsonicSearch => {
                let r = &self.subsonic.search_results;
                r.artists.len() + r.albums.len() + r.tracks.len()
            }
            PickerId::SubsonicAlbums => self.subsonic.albums.len(),
            PickerId::SubsonicAlbumTracks => self.subsonic.album_tracks.len(),
            PickerId::SubsonicSetup => 3,
            PickerId::PodcastFeeds => self.podcast.feeds.len(),
            PickerId::PodcastEpisodes => self.podcast.episodes.len(),
            PickerId::PodcastSubscribe => 1,
            PickerId::RadioSearch => self.radio.search.len(),
            PickerId::RadioTop => self.radio.top.len(),
            PickerId::RadioBrowse => 2,
            PickerId::RadioBrowseList => match self.radio.browse_kind {
                RadioBrowseKind::Tags => self.radio.browse_tags.len(),
                RadioBrowseKind::Countries => self.radio.browse_countries.len(),
            },
            PickerId::RadioBrowseStations => self.radio.browse_stations.len(),
            _ => 0,
        }
    }

    fn help_picker_total(&self) -> usize {
        HELP_LINES.len()
    }

    /// Move the top picker's selection by one (wrapping), clamped to the
    /// current item count.
    fn move_picker_selection(&mut self, down: bool) {
        let count = self.picker_item_count();
        if count == 0 {
            return;
        }
        if let Some(top) = self.pickers.top_mut() {
            if down {
                top.selected = if top.selected >= count.saturating_sub(1) {
                    0
                } else {
                    top.selected + 1
                };
            } else if top.selected == 0 {
                top.selected = count - 1;
            } else {
                top.selected -= 1;
            }
        }
    }

    /// Track id of the Subsonic row currently highlighted in a Subsonic
    /// picker (used for the cover-art preview).
    fn current_subsonic_track_id(&self) -> Option<String> {
        let top = self.pickers.top()?;
        let rows_before_tracks =
            self.subsonic.search_results.artists.len() + self.subsonic.search_results.albums.len();
        match top.id {
            PickerId::SubsonicSearch => {
                if top.selected >= rows_before_tracks {
                    self.subsonic
                        .search_results
                        .tracks
                        .get(top.selected - rows_before_tracks)
                        .map(|t| t.id.clone())
                } else {
                    None
                }
            }
            PickerId::SubsonicAlbumTracks => self
                .subsonic
                .album_tracks
                .get(top.selected)
                .map(|t| t.id.clone()),
            _ => None,
        }
    }

    /// Resolve a left-click against the zones registered by `ui::render`
    ///. A single click moves the selection; a double-click on
    /// the same row activates it exactly as Enter would.  Clicks outside an
    /// open picker panel close it.
    async fn handle_click(&mut self, x: u16, y: u16) {
        let Some(zone) = self.mouse_map.hit_test(x, y) else {
            // No interactive row under the cursor.
            if self.pickers.is_open() {
                let inside = self
                    .mouse_map
                    .picker_area
                    .is_some_and(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom());
                if !inside {
                    self.close_top_picker_with_cleanup();
                }
            }
            return;
        };

        match zone {
            MouseZone::PickerItem(i) => {
                if self.mouse_map.is_double_click(zone) {
                    let key = event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                    self.handle_key(key).await;
                } else if let Some(top) = self.pickers.top_mut() {
                    top.selected = i;
                    top.viewport_offset = top.viewport_offset.min(i);
                }
            }
            MouseZone::ListItem(i) => {
                let double = self.mouse_map.is_double_click(zone);
                self.library_pane_focus = false;
                self.set_list_pos(i);
                if double {
                    let key = event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                    self.handle_key(key).await;
                }
            }
        }
    }

    /// Handle a single terminal event. Returns false when the app should quit.
    async fn handle_terminal_event(&mut self, event: event::Event) -> bool {
        match event {
            event::Event::Key(key) => {
                if key.kind == KeyEventKind::Press
                    && (!self.handle_key(key).await || self.pending_quit)
                {
                    return false;
                }
            }
            event::Event::Paste(text) => {
                self.handle_paste(&text).await;
            }
            event::Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    let key = event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
                    self.handle_key(key).await;
                }
                MouseEventKind::ScrollDown => {
                    let key = event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
                    self.handle_key(key).await;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.handle_click(mouse.column, mouse.row).await;
                }
                _ => {}
            },
            event::Event::FocusGained | event::Event::FocusLost | event::Event::Resize(_, _) => {}
        }
        true
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        // Ctrl+Z: suspend to background (pass-through SIGTSTP)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('z') {
            self.pending_suspend = true;
            return false;
        }
        // Reset pending_motion if the key is not 'g'
        if key.code != KeyCode::Char('g') {
            self.pending_motion = None;
        }
        // If an picker is open, Esc closes it; keys pass through to picker
        if self.pickers.is_open() {
            return match key.code {
                KeyCode::Esc => {
                    if self
                        .pickers
                        .top()
                        .is_some_and(|o| o.id == PickerId::SpotifyLink)
                        && self.spotify.oauth_pending
                    {
                        // Cancel the pending OAuth browser flow.
                        self.spotify.oauth_pending = false;
                        self.spotify.oauth_url = None;
                        self.spotify.oauth_error = None;
                        let c = self.client.clone();
                        tokio::spawn(async move {
                            let _ = c.spotify().oauth_cancel().await;
                        });
                        self.close_top_picker_with_cleanup();
                    } else if self
                        .pickers
                        .top()
                        .is_some_and(|o| o.id == PickerId::PlaylistSelect)
                        && self.playlist_creating
                    {
                        self.playlist_creating = false;
                        if let Some(top) = self.pickers.top_mut() {
                            top.query.clear();
                            top.selected = 0;
                            top.viewport_offset = 0;
                        }
                    } else {
                        self.close_top_picker_with_cleanup();
                    }
                    true
                }
                _ => {
                    // Pass key to picker handler
                    self.handle_picker_key(key).await;
                    true
                }
            };
        }

        match self.input_mode {
            InputMode::Searching => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    // Commit the filter without clearing it and play
                    // the highlighted row in the filtered list.
                    self.play_filtered_highlighted();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                _ => {}
            },
            InputMode::Normal => {
                // If a prompt is pending, intercept keys
                if let Some(prompt) = self.pending_prompt.take() {
                    let is_confirm = prompt.confirm_keys.contains(&key.code);
                    let is_cancel = prompt.cancel_keys.contains(&key.code);
                    if is_confirm {
                        match prompt.prompt_type {
                            PromptType::DeleteTrack(track_id) => {
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::RemoveTrack(track_id)).await;
                            }
                            PromptType::MultiselectAddToQueue => {
                                let tracks = self.filtered_tracks();
                                let indices: Vec<usize> =
                                    self.selected_indices.iter().copied().collect();
                                let mut added = 0;
                                for idx in indices {
                                    if let Some(track) = tracks.get(idx) {
                                        let c = self.client.clone();
                                        let path = track.path.clone();
                                        tokio::spawn(async move {
                                            let _ = c.queue().add(&path, None).await;
                                        });
                                        added += 1;
                                    }
                                }
                                self.selected_indices.clear();
                                self.multiselect_mode = false;
                                self.fetch_queue().await;
                                self.footer_notification = Some((
                                    format!("Added {added} track(s) to queue"),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                            PromptType::MultiselectAddToPlaylist => {
                                let tracks = self.filtered_tracks();
                                let indices: Vec<i64> = self
                                    .selected_indices
                                    .iter()
                                    .filter_map(|i| tracks.get(*i).map(|t| t.id))
                                    .collect();
                                if !indices.is_empty() {
                                    self.pending_playlist_track_ids = indices;
                                    self.selected_indices.clear();
                                    self.multiselect_mode = false;
                                    self.playlist_creating = false;
                                    self.pickers.open(PickerId::PlaylistSelect);
                                }
                            }
                            PromptType::MultiselectDelete(_) | PromptType::None => {}
                        }
                    } else if is_cancel {
                        // Just drop the prompt
                    } else {
                        // Put the prompt back if key wasn't recognized
                        self.pending_prompt = Some(prompt);
                    }
                    return true;
                }
                // Health panel: Esc closes it
                if self.show_health_panel && key.code == KeyCode::Esc {
                    self.show_health_panel = false;
                    return true;
                }
                // Handle gg (vim-style double-press) for jump to start
                if key.code == KeyCode::Char('g') && !self.library_pane_focus {
                    if self.pending_motion == Some('g') {
                        // Second 'g': execute jump to start
                        self.pending_motion = None;
                        self.set_list_pos(0);

                        return true;
                    } else {
                        // First 'g': wait for second press
                        self.pending_motion = Some('g');
                        return true;
                    }
                }
                // In multiselect mode, Tab toggles selection and advances
                if key.code == KeyCode::Tab && self.multiselect_mode && !self.library_pane_focus {
                    let pos = self.list_pos();
                    if self.selected_indices.contains(&pos) {
                        self.selected_indices.remove(&pos);
                    } else {
                        self.selected_indices.insert(pos);
                    }
                    let max = self.filtered_tracks().len().saturating_sub(1);
                    self.set_list_pos((pos + 1).min(max));
                    let count = self.selected_indices.len();
                    self.notify_typed(
                        "System",
                        format!("{count} selected"),
                        NotificationKind::Info,
                        true,
                        NotifType::Prefs,
                    );
                    return true;
                }
                match self.keybindings.dispatch(key, KeyContext::Normal) {
                    Some(KeyboardAction::Quit) => {
                        if self.browse_detail.is_some() {
                            self.browse_detail = None;
                            self.set_list_pos(0);
                            if self.library_category == 5 {
                                self.spotify.playlist_tracks_cache.clear();
                            }
                        } else {
                            return false;
                        }
                    }
                    Some(KeyboardAction::QuitDaemon) => {
                        let c = self.client.clone();
                        // Await the daemon's reply so the quit request is
                        // actually delivered before the TUI exits. The daemon
                        // replies Ok then shuts down ~200ms later.
                        let _ = tokio::time::timeout(Duration::from_millis(1500), c.quit()).await;
                        return false;
                    }
                    Some(KeyboardAction::NextPane) => {
                        self.cycle_pane_focus(true);
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::PrevPane) => {
                        self.cycle_pane_focus(false);
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::OpenOverlay(id)) => {
                        self.pickers.open(id);
                        self.dismiss_track_popup();
                        self.on_picker_opened(id);
                    }
                    Some(KeyboardAction::ToggleHelp) => {
                        if self.pickers.top().is_some_and(|o| o.id == PickerId::Help) {
                            self.pickers.close_top();
                        } else {
                            self.pickers.open(PickerId::Help);
                            self.dismiss_track_popup();
                        }
                    }
                    Some(KeyboardAction::HideHelpBar) => {
                        self.hide_help_bar = !self.hide_help_bar;
                    }
                    Some(KeyboardAction::PlayPause) => {
                        self.set_last_action("Play/Pause");
                        match self.state.status {
                            PlaybackStatus::Playing => {
                                self.send_high(TuiCommand::Pause);
                            }
                            PlaybackStatus::Paused => {
                                self.send_high(TuiCommand::PlayPause);
                            }
                            PlaybackStatus::Stopped => {
                                if !self.queue.cache.is_empty() {
                                    let idx = self.queue.cursor.min(self.queue.cache.len() - 1);
                                    let path = self.queue.cache[idx].path.clone();
                                    self.send_high(TuiCommand::Play(path));
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::Next) => {
                        self.set_last_action("Next");
                        if self.multiselect_mode && !self.selected_indices.is_empty() {
                            let indices: Vec<usize> =
                                self.selected_indices.iter().copied().collect();
                            let count = indices.len();
                            self.pending_prompt = Some(PendingPrompt {
                                message: format!("Queue {count} selected track(s)? [y/N]"),
                                confirm_keys: vec![
                                    KeyCode::Char('y'),
                                    KeyCode::Char('Y'),
                                    KeyCode::Enter,
                                ],
                                cancel_keys: vec![
                                    KeyCode::Char('n'),
                                    KeyCode::Char('N'),
                                    KeyCode::Esc,
                                    KeyCode::Char('q'),
                                ],
                                prompt_type: PromptType::MultiselectAddToQueue,
                            });
                        } else {
                            self.send_high(TuiCommand::Next);
                        }
                    }
                    Some(KeyboardAction::Prev) => {
                        self.set_last_action("Previous");
                        if self.multiselect_mode && !self.selected_indices.is_empty() {
                            let indices: Vec<usize> =
                                self.selected_indices.iter().copied().collect();
                            let count = indices.len();
                            self.pending_prompt = Some(PendingPrompt {
                                message: format!("Queue {count} selected track(s)? [y/N]"),
                                confirm_keys: vec![
                                    KeyCode::Char('y'),
                                    KeyCode::Char('Y'),
                                    KeyCode::Enter,
                                ],
                                cancel_keys: vec![
                                    KeyCode::Char('n'),
                                    KeyCode::Char('N'),
                                    KeyCode::Esc,
                                    KeyCode::Char('q'),
                                ],
                                prompt_type: PromptType::MultiselectAddToQueue,
                            });
                        } else {
                            self.send_high(TuiCommand::Prev);
                        }
                    }
                    Some(KeyboardAction::Stop) => {
                        self.set_last_action("Stop");
                        self.send_high(TuiCommand::Stop);
                    }
                    Some(KeyboardAction::VolumeUp) => {
                        let new_vol = (self.state.volume + 5).min(MAX_VOLUME);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify_volume(new_vol);
                    }
                    Some(KeyboardAction::VolumeDown) => {
                        self.set_last_action("Volume Down");
                        let new_vol = self.state.volume.saturating_sub(5);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify_volume(new_vol);
                    }
                    Some(KeyboardAction::SpeedUp) => {
                        // Round to nearest 0.25 so the step stays predictable.
                        let new_speed = ((self.state.audio.speed + 0.25) * 4.0).ceil() / 4.0;
                        let new_speed = new_speed.min(MAX_SPEED);
                        self.set_last_action(&format!("Speed {:.2}x", new_speed));
                        self.send_high(TuiCommand::SetSpeed(new_speed));
                    }
                    Some(KeyboardAction::SpeedDown) => {
                        self.set_last_action("Speed Down");
                        let new_speed = ((self.state.audio.speed - 0.25) * 4.0).ceil() / 4.0;
                        let new_speed = new_speed.max(MIN_SPEED);
                        self.send_high(TuiCommand::SetSpeed(new_speed));
                    }
                    Some(KeyboardAction::ToggleLowPower) => {
                        self.set_last_action("Toggle Low-Power");
                        self.send_high(TuiCommand::SetLowPower(!self.state.low_power));
                        let msg = if self.state.low_power {
                            "Leaving low-power mode"
                        } else {
                            "Low-power mode (playback paused)"
                        };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::SeekForward) => {
                        self.set_last_action("Seek Forward");
                        self.accumulate_seek(5.0);
                    }
                    Some(KeyboardAction::SeekBackward) => {
                        self.set_last_action("SeekBackward");
                        self.accumulate_seek(-5.0);
                    }
                    Some(KeyboardAction::ToggleMute) => {
                        self.set_last_action("Toggle Mute");
                        self.send_high(TuiCommand::ToggleMute);
                        let msg = if self.state.mute { "Unmuted" } else { "Muted" };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::CycleRepeat) => {
                        self.set_last_action("Cycle Repeat");
                        let new_mode = match self.state.repeat {
                            RepeatMode::Off => RepeatMode::One,
                            RepeatMode::One => RepeatMode::All,
                            RepeatMode::All => RepeatMode::Off,
                        };
                        self.send_high(TuiCommand::CycleRepeat(new_mode));
                        self.footer_notification = Some((
                            format!("Repeat: {:?}", new_mode),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleShuffle) => {
                        // In a Spotify playlist drill-down, `S` (shift-s) means
                        // "shuffle play this playlist" instead of toggling the
                        // global queue shuffle.
                        if self.in_spotify_playlist() && !self.library_pane_focus {
                            let playlist_id = self.browse_detail.clone().unwrap_or_default();
                            let c = self.client.clone();
                            let ipc_tx2 = self.ipc_tx.clone();
                            self.footer_notification = Some((
                                "Shuffling playlist…".to_string(),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                            tokio::spawn(async move {
                                match c.spotify().play_all(&playlist_id, true).await {
                                    Ok(()) => {
                                        let _ = ipc_tx2.send(IpcResult::Notification(
                                            "Spotify".to_string(),
                                            "Shuffling playlist".to_string(),
                                            NotificationKind::Success,
                                            NotifType::Spotify,
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = ipc_tx2.send(IpcResult::Error(format!(
                                            "Spotify shuffle failed: {e}"
                                        )));
                                    }
                                }
                            });
                            return true;
                        }
                        self.set_last_action("Toggle Shuffle");
                        self.send_high(TuiCommand::ToggleShuffle);
                        let msg = if self.state.shuffle {
                            "Shuffle OFF"
                        } else {
                            "Shuffle ON"
                        };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleFavourite) => {
                        self.set_last_action("Toggle Favourite");
                        if self.library_pane_focus {
                            return true;
                        }
                        let (ids, label) = match self.library_category {
                            2 => {
                                // Album row: all tracks in the album
                                if let Some((name, _)) = self.unique_albums().get(self.list_pos()) {
                                    let ids: Vec<i64> = self
                                        .tracks_cache
                                        .iter()
                                        .filter(|t| {
                                            let album: &str = if t.album.is_empty() {
                                                "Unknown Album"
                                            } else {
                                                &t.album
                                            };
                                            album == name
                                        })
                                        .map(|t| t.id)
                                        .collect();
                                    (ids, name.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            3 => {
                                // Artist row: all tracks by the artist
                                if let Some((name, _)) = self.unique_artists().get(self.list_pos())
                                {
                                    let ids: Vec<i64> = self
                                        .tracks_cache
                                        .iter()
                                        .filter(|t| {
                                            let artist: &str = if t.artist.is_empty() {
                                                "Unknown Artist"
                                            } else {
                                                &t.artist
                                            };
                                            artist == name
                                        })
                                        .map(|t| t.id)
                                        .collect();
                                    (ids, name.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            4 => {
                                // Playlist row (drill-down open): all tracks in the playlist
                                if self.browse_detail.is_some() {
                                    let ids: Vec<i64> =
                                        self.filtered_tracks().iter().map(|t| t.id).collect();
                                    (ids, "Playlist".to_string())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            _ => {
                                // Track row (flat list / detail / Liked): toggle the highlighted track
                                let filtered = self.filtered_tracks();
                                if let Some(t) = filtered.get(self.list_pos()) {
                                    (vec![t.id], t.title.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                        };
                        if ids.is_empty() {
                            return true;
                        }
                        let all_fav = self
                            .tracks_cache
                            .iter()
                            .filter(|t| ids.contains(&t.id))
                            .all(|t| t.favourite);
                        let new_fav = !all_fav;
                        for t in &mut self.tracks_cache {
                            if ids.contains(&t.id) {
                                t.favourite = new_fav;
                            }
                        }
                        let tx = self.cmd_tx();
                        for id in ids {
                            let _ = tx
                                .send(if new_fav {
                                    TuiCommand::AddFavourite(id)
                                } else {
                                    TuiCommand::RemoveFavourite(id)
                                })
                                .await;
                        }
                        if !label.is_empty() {
                            let verb = if new_fav { "added to" } else { "removed from" };
                            self.notify_typed(
                                "System",
                                format!("{label}: {verb} favourites"),
                                NotificationKind::Info,
                                false,
                                NotifType::Playback,
                            );
                        }
                    }
                    Some(KeyboardAction::ClearQueue) => {
                        self.set_last_action("Clear Queue");
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::QueueClear).await;
                        self.footer_notification = Some((
                            "Queue cleared".to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleVisualizer) => {
                        self.visualizer.toggle();
                        let state = if self.visualizer.is_enabled() {
                            "ON"
                        } else {
                            "OFF"
                        };
                        self.footer_notification = Some((
                            format!("Visualizer: {}", state),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleTheme) => {
                        self.toggle_theme();
                    }
                    Some(KeyboardAction::CycleSort) => {
                        self.cycle_track_sort();
                    }
                    Some(KeyboardAction::CheckHealth) => {
                        self.send_high(TuiCommand::CheckHealth);
                    }
                    Some(KeyboardAction::FocusLeft) => {
                        if self.lyrics.show {
                            if self.lyrics.pane_focus {
                                // lyrics → right (track) pane
                                self.lyrics.pane_focus = false;
                                self.lyrics.manual_scroll = false;
                                self.library_pane_focus = false;
                            } else {
                                self.library_pane_focus = true;
                            }
                        } else {
                            self.library_pane_focus = true;
                        }
                    }
                    Some(KeyboardAction::FocusRight) => {
                        if self.lyrics.show {
                            if self.library_pane_focus {
                                // left → right (track) pane
                                self.library_pane_focus = false;
                                self.update_track_popup();
                            } else {
                                // right pane → lyrics pane
                                self.lyrics.pane_focus = true;
                            }
                        } else {
                            self.library_pane_focus = false;
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::Back) => {
                        if self.lyrics.pane_focus {
                            // Exit lyrics focus back to the track pane.
                            self.lyrics.pane_focus = false;
                            self.lyrics.manual_scroll = false;
                        } else {
                            let is_narrow = self.terminal_cols < 60;
                            if is_narrow && self.lyrics.show {
                                self.lyrics.show = false;
                            } else if self.browse_detail.is_some() {
                                self.browse_detail = None;
                                self.set_list_pos(0);
                                if self.library_category == 5 {
                                    self.spotify.playlist_tracks_cache.clear();
                                }
                            } else if !self.library_pane_focus {
                                self.library_pane_focus = true;
                            }
                        }
                    }
                    Some(KeyboardAction::FetchLyrics) => {
                        self.lyrics.show = !self.lyrics.show;
                        if !self.lyrics.show {
                            self.lyrics.pane_focus = false;
                            self.lyrics.manual_scroll = false;
                        }
                        if self.lyrics.show
                            && self.lyrics.current.is_none()
                            && !self.lyrics.fetching
                        {
                            self.lyrics.fetching = true;
                            self.send_high(TuiCommand::FetchLyrics);
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::MoveUp) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            self.lyrics.scroll = self.lyrics.scroll.saturating_sub(1);
                        } else if self.library_pane_focus {
                            let new_cat = self.library_category.saturating_sub(1);
                            if new_cat != self.library_category {
                                self.reset_library_view(new_cat, None);
                            }
                        } else {
                            self.set_list_pos(self.list_pos().saturating_sub(1));
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::MoveDown) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = (self.lyrics.scroll + 1).min(max);
                        } else if self.library_pane_focus {
                            let new_cat =
                                (self.library_category + 1).min(LIBRARY_CATEGORIES.len() - 1);
                            if new_cat != self.library_category {
                                self.reset_library_view(new_cat, None);
                            }
                        } else {
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos((self.list_pos() + 1).min(max_list));
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                        }
                    }
                    Some(KeyboardAction::PageUp) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            self.lyrics.scroll = self.lyrics.scroll.saturating_sub(page);
                        } else if !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            self.set_list_pos(self.list_pos().saturating_sub(page));
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::PageDown) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = (self.lyrics.scroll + page).min(max);
                        } else if !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos((self.list_pos() + page).min(max_list));
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                        }
                    }
                    Some(KeyboardAction::MultiselectUp) => {
                        if self.multiselect_mode && !self.library_pane_focus {
                            let pos = self.list_pos().saturating_sub(1);
                            self.set_list_pos(pos);
                            self.update_track_popup();
                            self.selected_indices.insert(pos);
                        }
                    }
                    Some(KeyboardAction::MultiselectDown) => {
                        if self.multiselect_mode && !self.library_pane_focus {
                            let max_list = self.library_list_len().saturating_sub(1);
                            let pos = (self.list_pos() + 1).min(max_list);
                            self.set_list_pos(pos);
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                            self.selected_indices.insert(pos);
                        }
                    }
                    Some(KeyboardAction::Top) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            self.lyrics.scroll = 0;
                        } else if !self.library_pane_focus {
                            self.set_list_pos(0);
                        }
                    }
                    Some(KeyboardAction::Bottom) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = max;
                        } else if !self.library_pane_focus {
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos(max_list);
                        }
                    }
                    Some(KeyboardAction::Select) => {
                        {
                            if self.library_pane_focus {
                                self.library_pane_focus = false;
                            } else if self.browse_detail.is_some() {
                                // In detail view: play the selected track of
                                // the rendered right-pane list.
                                if self.library_category == 5 {
                                    // Spotify playlist drill-down: rows 0/1 are
                                    // virtual actions (Play All / Shuffle), rows
                                    // 2+ resolve their track to a playable stream.
                                    let pos = self.list_pos();
                                    if pos < Self::SPOTIFY_PLAYLIST_ACTION_ROWS {
                                        let shuffle = pos == 1;
                                        let playlist_id =
                                            self.browse_detail.clone().unwrap_or_default();
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        tokio::spawn(async move {
                                            match c.spotify().play_all(&playlist_id, shuffle).await
                                            {
                                                Ok(()) => {
                                                    let _ = ipc_tx2.send(IpcResult::Notification(
                                                        "Spotify".to_string(),
                                                        if shuffle {
                                                            "Shuffling playlist".to_string()
                                                        } else {
                                                            "Playing playlist".to_string()
                                                        },
                                                        NotificationKind::Success,
                                                        NotifType::Spotify,
                                                    ));
                                                }
                                                Err(e) => {
                                                    let _ = ipc_tx2.send(IpcResult::Error(
                                                        format!("Spotify play-all failed: {e}"),
                                                    ));
                                                }
                                            }
                                        });
                                        return true;
                                    }
                                    if let Some(track) = self.selected_spotify_track().cloned() {
                                        // Resolve the track to a playable local
                                        // stream and enqueue it.
                                        let playlist_id =
                                            self.browse_detail.clone().unwrap_or_default();
                                        let track_index = track.index;
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        tokio::spawn(async move {
                                            match c
                                                .spotify()
                                                .resolve(&playlist_id, track_index)
                                                .await
                                            {
                                                Ok(()) => {
                                                    let _ = ipc_tx2.send(IpcResult::Notification(
                                                        "Spotify".to_string(),
                                                        "Spotify track resolved & queued"
                                                            .to_string(),
                                                        NotificationKind::Success,
                                                        NotifType::Spotify,
                                                    ));
                                                }
                                                Err(e) => {
                                                    let _ = ipc_tx2.send(IpcResult::Error(
                                                        format!("Spotify resolve failed: {e}"),
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                } else {
                                    // Local categories (All/Liked/Album/Artist/
                                    // Playlist): the rendered rows are exactly
                                    // filtered_tracks(), so play that row.
                                    self.play_filtered_highlighted();
                                }
                            } else if self.library_category == 2 {
                                // Albums: select album → show its tracks
                                let albums = self.unique_albums();
                                let pos = self.list_pos();
                                if pos < albums.len() {
                                    self.browse_detail = Some(albums[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 3 {
                                // Artists: select artist → show its tracks
                                let artists = self.unique_artists();
                                let pos = self.list_pos();
                                if pos < artists.len() {
                                    self.browse_detail = Some(artists[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 4 {
                                // Playlists: select playlist → show its tracks
                                if self.list_pos() < self.playlist_cache.len() {
                                    let playlist = self.playlist_cache[self.list_pos()].clone();
                                    self.browse_detail = Some(playlist.name.clone());
                                    self.set_list_pos(0);
                                    self.playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    tokio::spawn(async move {
                                        if let Ok(DaemonRes::Tracks { tracks }) =
                                            c.library().get_playlist_tracks(pid).await
                                        {
                                            let _ = ipc_tx2.send(IpcResult::PlaylistTracks(tracks));
                                        }
                                    });
                                }
                            } else if self.library_category == 5 {
                                // Spotify: select playlist → show its cached tracks
                                if self.list_pos() < self.spotify.playlists.len() {
                                    let playlist = self.spotify.playlists[self.list_pos()].clone();
                                    self.browse_detail = Some(playlist.id.clone());
                                    self.set_list_pos(0);
                                    self.spotify.playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    tokio::spawn(async move {
                                        match c.spotify().playlist_tracks(&pid).await {
                                            Ok(tracks) => {
                                                let _ =
                                                    ipc_tx2.send(IpcResult::SpotifyTracks(tracks));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx2.send(IpcResult::Error(format!(
                                                    "Spotify playlist load failed: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                            } else if self.library_category <= 1 {
                                // Default: play track from flat list (All Tracks / Liked)
                                self.play_filtered_highlighted();
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {
                        if !self.library_pane_focus {
                            let track_data = self
                                .filtered_tracks()
                                .get(self.list_pos())
                                .map(|t| (t.id, t.title.clone()));
                            if let Some((track_id, track_name)) = track_data {
                                self.pending_prompt = Some(PendingPrompt {
                                    message: format!("Delete \"{track_name}\"? [y/N]"),
                                    confirm_keys: vec![
                                        KeyCode::Char('y'),
                                        KeyCode::Char('Y'),
                                        KeyCode::Enter,
                                    ],
                                    cancel_keys: vec![
                                        KeyCode::Char('n'),
                                        KeyCode::Char('N'),
                                        KeyCode::Esc,
                                        KeyCode::Char('q'),
                                    ],
                                    prompt_type: PromptType::DeleteTrack(track_id),
                                });
                            }
                        }
                    }
                    Some(KeyboardAction::ToggleMultiselect) => {
                        if !self.library_pane_focus {
                            self.multiselect_mode = !self.multiselect_mode;
                            if !self.multiselect_mode {
                                self.selected_indices.clear();
                            }
                            let msg = if self.multiselect_mode {
                                "Multiselect ON"
                            } else {
                                "Multiselect OFF"
                            };
                            self.footer_notification = Some((
                                msg.to_string(),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                        }
                    }
                    Some(KeyboardAction::AddToQueue) => {
                        if !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<usize> =
                                if self.multiselect_mode && !self.selected_indices.is_empty() {
                                    self.selected_indices.iter().copied().collect()
                                } else {
                                    vec![self.list_pos()]
                                };
                            if self.multiselect_mode && !self.selected_indices.is_empty() {
                                let count = indices.len();
                                self.pending_prompt = Some(PendingPrompt {
                                    message: format!("Add {count} tracks to queue? [y/N]"),
                                    confirm_keys: vec![
                                        KeyCode::Char('y'),
                                        KeyCode::Char('Y'),
                                        KeyCode::Enter,
                                    ],
                                    cancel_keys: vec![
                                        KeyCode::Char('n'),
                                        KeyCode::Char('N'),
                                        KeyCode::Esc,
                                        KeyCode::Char('q'),
                                    ],
                                    prompt_type: PromptType::MultiselectAddToQueue,
                                });
                            } else {
                                let mut added = 0;
                                for idx in indices {
                                    if let Some(track) = tracks.get(idx) {
                                        let c = self.client.clone();
                                        let path = track.path.clone();
                                        tokio::spawn(async move {
                                            let _ = c.queue().add(&path, None).await;
                                        });
                                        added += 1;
                                    }
                                }
                                self.fetch_queue().await;
                                self.footer_notification = Some((
                                    format!("Added {added} track(s) to queue"),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                        }
                    }
                    Some(KeyboardAction::AddToPlaylist) => {
                        if !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<i64> =
                                if self.multiselect_mode && !self.selected_indices.is_empty() {
                                    self.selected_indices
                                        .iter()
                                        .filter_map(|i| tracks.get(*i).map(|t| t.id))
                                        .collect()
                                } else {
                                    tracks
                                        .get(self.list_pos())
                                        .map(|t| vec![t.id])
                                        .unwrap_or_default()
                                };
                            if self.multiselect_mode && !self.selected_indices.is_empty() {
                                let count = indices.len();
                                self.pending_prompt = Some(PendingPrompt {
                                    message: format!("Add {count} tracks to playlist? [y/N]"),
                                    confirm_keys: vec![
                                        KeyCode::Char('y'),
                                        KeyCode::Char('Y'),
                                        KeyCode::Enter,
                                    ],
                                    cancel_keys: vec![
                                        KeyCode::Char('n'),
                                        KeyCode::Char('N'),
                                        KeyCode::Esc,
                                        KeyCode::Char('q'),
                                    ],
                                    prompt_type: PromptType::MultiselectAddToPlaylist,
                                });
                            } else if !indices.is_empty() {
                                self.pending_playlist_track_ids = indices;
                                self.playlist_creating = false;
                                self.pickers.open(PickerId::PlaylistSelect);
                            }
                        }
                    }
                    Some(KeyboardAction::DeleteFromList) => {
                        if !self.library_pane_focus {
                            if self.library_category == 4 && self.browse_detail.is_some() {
                                // In playlist view: remove selected track from playlist
                                let filtered = self.filtered_tracks();
                                if let Some(track) = filtered.get(self.list_pos()) {
                                    let track_id = track.id;
                                    if let Some(pl) = self
                                        .playlist_cache
                                        .iter()
                                        .find(|p| self.browse_detail.as_deref() == Some(&p.name))
                                    {
                                        let playlist_id = pl.id;
                                        let tx = self.cmd_tx();
                                        let _ = tx
                                            .send(TuiCommand::RemoveFromPlaylist(
                                                playlist_id,
                                                track_id,
                                            ))
                                            .await;
                                        self.notify_typed(
                                            "System",
                                            "Removed from playlist",
                                            NotificationKind::Info,
                                            false,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                }
                            } else {
                                self.notify_typed(
                                    "System",
                                    "Remove from list only available in playlist view",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                            }
                        }
                    }
                    Some(KeyboardAction::JumpToEnd) => {
                        if !self.library_pane_focus {
                            let max = self.library_list_len().saturating_sub(1);
                            self.set_list_pos(max);
                        }
                    }
                    Some(KeyboardAction::EditMetadata) => {
                        if !self.library_pane_focus {
                            let track_data = {
                                let tracks = self.filtered_tracks();
                                tracks.get(self.list_pos()).map(|t| {
                                    (
                                        t.id,
                                        t.title.clone(),
                                        t.artist.clone(),
                                        t.album.clone(),
                                        t.genre.clone(),
                                        t.year,
                                        t.track_number,
                                    )
                                })
                            };
                            if let Some((id, title, artist, album, genre, year, track_num)) =
                                track_data
                            {
                                self.metadata.edit_track_id = Some(id);
                                self.metadata.fields = [
                                    title,
                                    artist,
                                    album,
                                    String::new(),
                                    genre,
                                    year.map_or(String::new(), |y| y.to_string()),
                                    track_num.map_or(String::new(), |n| n.to_string()),
                                ];
                                self.metadata.field_idx = 0;
                                self.pickers.open(PickerId::EditMetadata);
                                self.fetch_metadata_cover();
                            }
                        }
                    }
                    // Queue move actions: only handled in picker mode
                    Some(KeyboardAction::QueueMoveUp)
                    | Some(KeyboardAction::QueueMoveDown)
                    | Some(KeyboardAction::QueueMoveConfirm)
                    | Some(KeyboardAction::QueueMoveCancel) => {}
                    None => {
                        match key.code {
                            KeyCode::Char('q') => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.set_list_pos(0);
                                } else {
                                    return false;
                                }
                            }
                            KeyCode::Esc => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.set_list_pos(0);
                                }
                            }
                            KeyCode::Char('S') => {
                                // Sync covers for tracks missing cover art
                                self.notify_titled(
                                    "Library",
                                    "Syncing covers...",
                                    NotificationKind::Info,
                                    true,
                                    NotifType::Library,
                                );
                                spawn_sync_and_wait(
                                    self.client.clone(),
                                    SyncKind::Covers,
                                    "Covers",
                                    self.ipc_tx.clone(),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        true
    }

    /// Close the top picker, clearing per-picker state that Esc must reset
    /// (same cleanup for arrow-key closes, ).
    fn close_top_picker_with_cleanup(&mut self) {
        if let Some(top) = self.pickers.top() {
            match top.id {
                PickerId::SleepTimer => self.sleep_timer.remaining = None,
                PickerId::SpotifyLink => {
                    // Closing the link picker ends any pending/cancelled flow.
                    self.spotify.oauth_pending = false;
                    self.spotify.oauth_url = None;
                    self.spotify.oauth_error = None;
                }
                PickerId::SpotifySearch => self.spotify.search_results.clear(),
                PickerId::EditMetadata => {
                    self.metadata.cover = None;
                    self.metadata.cover_stateful = None;
                    self.metadata.edit_track_id = None;
                    self.metadata.cover_fetch_gen = None;
                }
                PickerId::SearchLibrary => {
                    // Robust: clear preview dedup state so reopen does not retain
                    // stale fetch ids/gens and show blank until selection moves.
                    self.clear_search_previews();
                    self.clear_preview();
                    self.clear_popup_cover();
                }
                PickerId::Queue => {
                    self.clear_preview();
                }
                PickerId::PlaylistTrackSelect => {
                    self.pending_playlist_id = None;
                    self.selected_playlist_track_ids.clear();
                }
                _ => {}
            }
        }
        self.pickers.close_top();
    }

    /// Add the tracks highlighted in the post-create multi-select picker to the
    /// pending playlist, then close the picker.
    fn commit_playlist_selection(&mut self) {
        let Some(pid) = self.pending_playlist_id else {
            self.close_top_picker_with_cleanup();
            return;
        };
        let track_ids: Vec<i64> = self.selected_playlist_track_ids.iter().copied().collect();
        if track_ids.is_empty() {
            self.notify_typed(
                "System",
                "No tracks selected — playlist stays empty",
                NotificationKind::Info,
                false,
                NotifType::NowPlaying,
            );
            self.close_top_picker_with_cleanup();
            self.pending_playlist_id = None;
            self.selected_playlist_track_ids.clear();
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = client.library().add_to_playlist(pid, track_ids).await {
                let _ = ipc_tx.send(IpcResult::Error(format!("Failed to add tracks: {e}")));
                return;
            }
            let _ = ipc_tx.send(IpcResult::Notification(
                "Playlist".to_string(),
                "Tracks added to playlist".to_string(),
                NotificationKind::Success,
                NotifType::NowPlaying,
            ));
        });
        self.close_top_picker_with_cleanup();
        self.pending_playlist_id = None;
        self.selected_playlist_track_ids.clear();
    }

    async fn handle_picker_key(&mut self, key: event::KeyEvent) {
        // While the OAuth browser flow is pending, the SpotifyLink picker is in
        // a waiting state; ignore all key input except Esc (handled in
        // handle_key) so the user can't mutate the now-irrelevant input.
        if self.spotify.oauth_pending
            && self
                .pickers
                .top()
                .is_some_and(|o| o.id == PickerId::SpotifyLink)
        {
            return;
        }

        let tx = self.cmd_tx();

        // Ctrl+D in SpotifySearch picker: download the selected track via YouTube
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('d')
            && self
                .pickers
                .top()
                .is_some_and(|o| o.id == PickerId::SpotifySearch)
        {
            if !self.spotify.search_results.is_empty() {
                let idx = self
                    .pickers
                    .top()
                    .map_or(0, |o| o.selected)
                    .min(self.spotify.search_results.len() - 1);
                let (playlist_id, _, track) = self.spotify.search_results[idx].clone();
                let track_index = track.index;
                let can_stream = track.uri.is_some();
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                self.pickers.close_top();
                tokio::spawn(async move {
                    // Web hits are not in any synced playlist: resolve by
                    // metadata + known URI (streams natively on Premium), not
                    // by playlist index.
                    let res = if playlist_id == "web" {
                        c.spotify()
                            .resolve_track(
                                &track.name,
                                &track.artists,
                                track.album.as_deref().unwrap_or(""),
                                track.uri.clone(),
                            )
                            .await
                    } else {
                        c.spotify().resolve(&playlist_id, track_index).await
                    };
                    match res {
                        Ok(()) => {
                            let _ = ipc_tx.send(IpcResult::Notification(
                                "Spotify".to_string(),
                                format!(
                                    "Queued: {} - {}{}",
                                    track.artists,
                                    track.name,
                                    if can_stream { "" } else { " (YouTube)" }
                                ),
                                NotificationKind::Success,
                                NotifType::Spotify,
                            ));
                        }
                        Err(e) => {
                            let _ = ipc_tx
                                .send(IpcResult::Error(format!("Spotify resolve failed: {e}")));
                        }
                    }
                });
            } else {
                self.notify_typed(
                    "System",
                    "No track selected to download",
                    NotificationKind::Info,
                    false,
                    NotifType::Downloads,
                );
            }
            return;
        }

        // PlaylistTrackSelect picker (post-create multi-select):
        //   Space / Tab    toggle the highlighted track
        //   Ctrl+Enter     commit highlighted tracks to the playlist
        //   Esc            cancel (handled by the common Esc path)
        if self
            .pickers
            .top()
            .is_some_and(|o| o.id == PickerId::PlaylistTrackSelect)
        {
            let is_ctrl_enter = key.modifiers.contains(KeyModifiers::CONTROL)
                && (key.code == KeyCode::Enter || key.code == KeyCode::Char('m'));
            let is_toggle = key.code == KeyCode::Char(' ') || key.code == KeyCode::Tab;
            if is_ctrl_enter {
                self.commit_playlist_selection();
                return;
            }
            if is_toggle {
                if let Some(top) = self.pickers.top()
                    && let Some(track) = self.tracks_cache.get(top.selected)
                {
                    let id = track.id;
                    if !self.selected_playlist_track_ids.remove(&id) {
                        self.selected_playlist_track_ids.insert(id);
                    }
                }
                return;
            }
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Queue)) {
            // Queue move mode: Ctrl+j/k to move, Enter to confirm, Esc to cancel
            if self.queue.move_index.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        // Cancel move mode
                        self.queue.move_index = None;
                        self.queue.move_target = 0;
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected = self.queue.move_target;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        // Confirm move
                        if let Some(from_idx) = self.queue.move_index {
                            let to_idx = self.queue.move_target;
                            if from_idx != to_idx && to_idx < self.queue.cache.len() {
                                self.send_high(TuiCommand::QueueMove(
                                    from_idx as u64,
                                    to_idx as u64,
                                ));
                            }
                        }
                        self.queue.move_index = None;
                        self.queue.move_target = 0;
                        return;
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Move down
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected =
                                (top.selected + 1).min(self.queue.cache.len().saturating_sub(1));
                            self.queue.move_target = top.selected;
                        }
                        return;
                    }
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Move up
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected = top.selected.saturating_sub(1);
                            self.queue.move_target = top.selected;
                        }
                        return;
                    }
                    _ => {}
                }
            } else {
                // Enter move mode when Ctrl+j or Ctrl+k is pressed
                match key.code {
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !self.queue.cache.is_empty() {
                            self.queue.move_index = Some(
                                self.pickers
                                    .top()
                                    .map(|o| o.selected)
                                    .unwrap_or(0)
                                    .min(self.queue.cache.len().saturating_sub(1)),
                            );
                            self.queue.move_target = self.queue.move_index.unwrap();
                            if let Some(top) = self.pickers.top_mut() {
                                top.selected = (top.selected + 1)
                                    .min(self.queue.cache.len().saturating_sub(1));
                                self.queue.move_target = top.selected;
                            }
                        }
                        return;
                    }
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !self.queue.cache.is_empty() {
                            self.queue.move_index = Some(
                                self.pickers
                                    .top()
                                    .map(|o| o.selected)
                                    .unwrap_or(0)
                                    .min(self.queue.cache.len().saturating_sub(1)),
                            );
                            self.queue.move_target = self.queue.move_index.unwrap();
                            if let Some(top) = self.pickers.top_mut() {
                                top.selected = top.selected.saturating_sub(1);
                                self.queue.move_target = top.selected;
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::SleepTimer)) {
            if self.sleep_timer.input_mode {
                match key.code {
                    KeyCode::Esc => {
                        self.sleep_timer.input_mode = false;
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Enter => {
                        if let Ok(m) = self.sleep_timer.input_buf.parse::<u32>() {
                            self.sleep_timer.minutes = m.min(180);
                        }
                        self.sleep_timer.input_mode = false;
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Backspace => {
                        self.sleep_timer.input_buf.pop();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        self.sleep_timer.input_buf.push(c);
                    }
                    _ => {}
                }
                return;
            }
            match key.code {
                KeyCode::Esc => {
                    self.sleep_timer.remaining = None;
                    self.sleep_timer.minutes = 30;
                    self.sleep_timer.input_mode = false;
                    self.sleep_timer.input_buf.clear();
                    self.pickers.close_top();
                    return;
                }
                // Arrows are navigation only: Left/Right leave the picker like
                // Esc. Minute stepping stays on h/l and +/-.
                KeyCode::Left | KeyCode::Right => {
                    self.sleep_timer.remaining = None;
                    self.sleep_timer.minutes = 30;
                    self.sleep_timer.input_mode = false;
                    self.sleep_timer.input_buf.clear();
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Char('h') => {
                    self.sleep_timer.minutes = self.sleep_timer.minutes.saturating_sub(5);
                    return;
                }
                KeyCode::Char('l') => {
                    self.sleep_timer.minutes = (self.sleep_timer.minutes + 5).min(180);
                    return;
                }
                KeyCode::Char('-') => {
                    self.sleep_timer.minutes = self.sleep_timer.minutes.saturating_sub(1);
                    return;
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.sleep_timer.minutes = (self.sleep_timer.minutes + 1).min(180);
                    return;
                }
                KeyCode::Enter => {
                    let mins = self.sleep_timer.minutes;
                    self.sleep_timer.remaining = Some(mins as u64);
                    self.send_high(TuiCommand::SetSleepTimer(mins));
                    self.footer_notification = Some((
                        format!("Sleep timer set: {} min", mins),
                        std::time::Instant::now() + std::time::Duration::from_secs(2),
                    ));
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Char('i') => {
                    self.sleep_timer.input_mode = true;
                    self.sleep_timer.input_buf.clear();
                    return;
                }
                KeyCode::Char('c') => {
                    self.sleep_timer.remaining = None;
                    self.send_high(TuiCommand::CancelSleepTimer);
                    self.footer_notification = Some((
                        "Sleep timer cancelled".to_string(),
                        std::time::Instant::now() + std::time::Duration::from_secs(2),
                    ));
                    return;
                }
                KeyCode::Up | KeyCode::Char('j') => {
                    let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = (top.selected + 1) % quick_opts.len();
                        self.sleep_timer.minutes = quick_opts[top.selected];
                    }
                    return;
                }
                KeyCode::Down | KeyCode::Char('k') => {
                    let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = if top.selected == 0 {
                            quick_opts.len() - 1
                        } else {
                            top.selected - 1
                        };
                        self.sleep_timer.minutes = quick_opts[top.selected];
                    }
                    return;
                }
                _ => {}
            }
            return;
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Settings)) {
            let settings_focus = self.settings_pane_focus;
            match key.code {
                KeyCode::Esc => {
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Tab => {
                    self.settings_pane_focus = !self.settings_pane_focus;
                    return;
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !settings_focus {
                        match self.settings_category {
                            1 => match self.settings_option {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.cycle_repeat(next).await;
                                    });
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.toggle_shuffle().await;
                                    });
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    });
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    });
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match self.settings_option {
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            tokio::spawn(async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                _ => {}
                            },
                            3 => {}
                            _ => {}
                        }
                    }
                    return;
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if !settings_focus {
                        match self.settings_category {
                            1 => match self.settings_option {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.cycle_repeat(next).await;
                                    });
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.toggle_shuffle().await;
                                    });
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    });
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    });
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match self.settings_option {
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            tokio::spawn(async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                _ => {}
                            },
                            3 => {}
                            _ => {}
                        }
                    }
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if settings_focus {
                        self.settings_category = self.settings_category.saturating_sub(1);
                        self.settings_option = 0;
                    } else {
                        self.settings_option = self.settings_option.saturating_sub(1);
                    }
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if settings_focus {
                        self.settings_category =
                            (self.settings_category + 1).min(NUM_SETTINGS_CATEGORIES - 1);
                        self.settings_option = 0;
                    } else {
                        let max = self.settings_options_for_category().saturating_sub(1);
                        self.settings_option = (self.settings_option + 1).min(max);
                    }
                    return;
                }
                KeyCode::Enter => {
                    if !settings_focus {
                        let opt = self.settings_option;
                        match self.settings_category {
                            0 => {
                                if opt == 1 {
                                    let current = self.cookie_file.clone();
                                    let new_path = if current.is_some() {
                                        None
                                    } else {
                                        let home = std::env::var("HOME").unwrap_or_default();
                                        Some(format!("{home}/.cookies/youtube.txt"))
                                    };
                                    let display =
                                        new_path.clone().unwrap_or_else(|| "(none)".to_string());
                                    self.cookie_file = new_path.clone();
                                    let c = self.client.clone();
                                    let cf = new_path;
                                    tokio::spawn(async move {
                                        let _ = c.yt().set_config(None, cf, None, None, None).await;
                                    });
                                    self.notify_typed(
                                        "System",
                                        format!("Cookie file: {display}"),
                                        NotificationKind::Info,
                                        false,
                                        NotifType::Prefs,
                                    );
                                }
                            }
                            1 => match opt {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.cycle_repeat(next).await;
                                    });
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.toggle_shuffle().await;
                                    });
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                2 => {
                                    self.pickers.open(PickerId::Crossfade);
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    });
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    });
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match opt {
                                0 => {
                                    self.pickers.open(PickerId::ThemePicker);
                                }
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                3 => {
                                    spawn_sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Covers,
                                        "Covers",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                4 => {
                                    spawn_sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Lyrics,
                                        "Lyrics",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                5 => {
                                    spawn_sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Metadata,
                                        "Metadata",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                6 => {
                                    self.pickers.open(PickerId::FooterPreset);
                                }
                                7 => {
                                    self.pickers.open(PickerId::VisualizerPreset);
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            tokio::spawn(async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                10 | 11 => {
                                    let what = if opt == 10 {
                                        CacheKind::Lyrics
                                    } else {
                                        CacheKind::Covers
                                    };
                                    let label = if opt == 10 { "lyrics" } else { "cover art" };
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        match c.clear_cache(what).await {
                                            Ok(()) => {
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Cache".to_string(),
                                                    format!("Cleared {label} cache"),
                                                    NotificationKind::Success,
                                                    NotifType::Prefs,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Clear {label} cache: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                                12 => {
                                    self.pickers.open(PickerId::NotificationSettings);
                                }
                                13 => {
                                    self.cycle_theme_mode();
                                }
                                _ => {}
                            },
                            3 => match opt {
                                0 => {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        match c.spotify().play_pause().await {
                                            Ok(status) => {
                                                let _ =
                                                    ipc_tx.send(IpcResult::SpotifyStatus(status));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify play/pause: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                                3 => {
                                    self.spotify.link_input.clear();
                                    self.spotify.oauth_port = "8990".to_string();
                                    self.spotify.link_field = 0;
                                    if let Some(cid) = get_secret(SPOTIFY_CLIENT_ID_KEY) {
                                        self.spotify.link_input = cid;
                                    }
                                    self.pickers.open(PickerId::SpotifyLink);
                                }
                                4 => {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        match c.spotify().sync().await {
                                            Ok(()) => {
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    "Spotify sync complete".to_string(),
                                                    NotificationKind::Success,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify sync: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                                5 => {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        match c.spotify().clear().await {
                                            Ok(status) => {
                                                let _ =
                                                    ipc_tx.send(IpcResult::SpotifyStatus(status));
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    "Account unlinked".to_string(),
                                                    NotificationKind::Info,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify unlink: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                                7 => {
                                    self.spotify.link_input.clear();
                                    self.spotify.oauth_port = "8990".to_string();
                                    self.spotify.link_field = 0;
                                    if let Some(cid) = get_secret(SPOTIFY_CLIENT_ID_KEY) {
                                        self.spotify.link_input = cid;
                                    }
                                    self.pickers.open(PickerId::SpotifyLink);
                                }
                                _ => {}
                            },
                            9 => {
                                self.hide_footer = !self.hide_footer;
                                save_prefs(&self.current_prefs());
                            }
                            _ => {}
                        }
                    }
                    return;
                }
                _ => {}
            }
            return;
        }

        // ─── Subsonic search picker ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::SubsonicSearch)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        if let Some(top) = self.pickers.top_mut() {
                            top.query.push(c);
                        }
                        self.subsonic.search_results = SubsonicSearchResults::default();
                        self.subsonic.cover_preview = None;
                    }
                }
                KeyCode::Backspace => {
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.pop();
                    }
                    self.subsonic.search_results = SubsonicSearchResults::default();
                }
                KeyCode::Enter => {
                    let r = &self.subsonic.search_results;
                    let n_artists = r.artists.len();
                    let n_albums = r.albums.len();
                    let n_tracks = r.tracks.len();
                    let has_results = n_artists + n_albums + n_tracks > 0;
                    let q = self
                        .pickers
                        .top()
                        .map_or(String::new(), |o| o.query.clone());
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if self.subsonic.search_pending {
                        return;
                    }
                    if q.is_empty() && !has_results {
                        return;
                    }
                    if !has_results {
                        self.subsonic_search(q);
                        return;
                    }
                    if sel >= n_artists && sel < n_artists + n_albums {
                        if let Some(album) = self
                            .subsonic
                            .search_results
                            .albums
                            .get(sel - n_artists)
                            .cloned()
                        {
                            self.subsonic.selected_album = Some(album);
                            self.subsonic.album_tracks.clear();
                            self.pickers.open(PickerId::SubsonicAlbumTracks);
                            let c = self.client.clone();
                            let ipc_tx = self.ipc_tx.clone();
                            let album_id = self
                                .subsonic
                                .selected_album
                                .as_ref()
                                .map(|a| a.id.clone())
                                .unwrap_or_default();
                            tokio::spawn(async move {
                                match c.subsonic().album_tracks(&album_id).await {
                                    Ok(t) => {
                                        let _ = ipc_tx.send(IpcResult::SubsonicAlbumTracks(t));
                                    }
                                    Err(e) => {
                                        self_err(&ipc_tx, format!("subsonic album failed: {e}"));
                                    }
                                }
                            });
                        }
                        return;
                    }
                    if sel < n_artists + n_albums + n_tracks {
                        let i = sel - n_artists - n_albums;
                        if let Some(track) = self.subsonic.search_results.tracks.get(i).cloned() {
                            let c = self.client.clone();
                            self.pickers.close_top();
                            tokio::spawn(async move {
                                let _ = c.subsonic().play(&track).await;
                            });
                        }
                    }
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                    if let Some(track_id) = self.current_subsonic_track_id() {
                        self.fetch_subsonic_cover(track_id);
                    }
                }
                _ => {}
            }
            return;
        }

        // ─── Subsonic album browser ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::SubsonicAlbums)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(album) = self.subsonic.albums.get(sel).cloned() {
                        let album_id = album.id.clone();
                        self.subsonic.selected_album = Some(album);
                        self.subsonic.album_tracks.clear();
                        self.pickers.open(PickerId::SubsonicAlbumTracks);
                        let c = self.client.clone();
                        let ipc_tx = self.ipc_tx.clone();
                        tokio::spawn(async move {
                            match c.subsonic().album_tracks(&album_id).await {
                                Ok(t) => {
                                    let _ = ipc_tx.send(IpcResult::SubsonicAlbumTracks(t));
                                }
                                Err(e) => {
                                    self_err(&ipc_tx, format!("subsonic album failed: {e}"));
                                }
                            }
                        });
                    }
                }
                KeyCode::Char('r') => {
                    self.subsonic.albums.clear();
                    self.subsonic.albums_pending = true;
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        match c.subsonic().albums(0, 200).await {
                            Ok(a) => {
                                let _ = ipc_tx.send(IpcResult::SubsonicAlbums(a));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("subsonic albums failed: {e}"));
                            }
                        }
                    });
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Subsonic album track list ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::SubsonicAlbumTracks)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let idx = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(track) = self.subsonic.album_tracks.get(idx).cloned() {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.subsonic().play(&track).await;
                        });
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(album_id) =
                        self.subsonic.selected_album.as_ref().map(|a| a.id.clone())
                    {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.subsonic().play_album(&album_id).await;
                        });
                    }
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                    if let Some(track_id) = self.current_subsonic_track_id() {
                        self.fetch_subsonic_cover(track_id);
                    }
                }
                _ => {}
            }
            return;
        }

        // ─── gtm setup service chooser ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Setup)) {
            match key.code {
                KeyCode::Up | KeyCode::Down => {
                    let n = 3;
                    self.setup.selection = (self.setup.selection as i32
                        + if key.code == KeyCode::Down { 1 } else { -1 })
                    .rem_euclid(n) as usize;
                }
                KeyCode::Enter => {
                    let (sel, service) = setup_selection(self);
                    let _ = sel;
                    self.pickers.close_top();
                    self.open_setup_picker(Some(service));
                }
                _ => {}
            }
            return;
        }

        // ─── Last.fm setup form ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::LastfmAuth)) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        match self.setup.lastfm_focus {
                            0 => self.setup.lastfm_api_key.push(c),
                            _ => self.setup.lastfm_api_secret.push(c),
                        }
                    }
                }
                KeyCode::Backspace => match self.setup.lastfm_focus {
                    0 => {
                        self.setup.lastfm_api_key.pop();
                    }
                    _ => {
                        self.setup.lastfm_api_secret.pop();
                    }
                },
                KeyCode::Tab => {
                    self.setup.lastfm_focus = (self.setup.lastfm_focus + 1) % 2;
                }
                KeyCode::Enter => {
                    let api_key = self.setup.lastfm_api_key.clone();
                    let api_secret = self.setup.lastfm_api_secret.clone();
                    if api_key.trim().is_empty() || api_secret.trim().is_empty() {
                        self.setup.lastfm_error = Some("API key and secret are required".into());
                        return;
                    }
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    self.setup.lastfm_pending = true;
                    self.setup.lastfm_error = None;
                    tokio::spawn(async move {
                        match c
                            .lastfm()
                            .set_config(true, Some(api_key), Some(api_secret), None, None, None)
                            .await
                        {
                            Ok(()) => match c.lastfm().auth_url().await {
                                Ok(url) => {
                                    let _ = webbrowser::open(&url);
                                    let _ = ipc_tx.send(IpcResult::LastfmAuthUrl(url));
                                }
                                Err(e) => {
                                    let _ = ipc_tx.send(IpcResult::LastfmAuthError(format!(
                                        "Last.fm auth URL failed: {e}"
                                    )));
                                }
                            },
                            Err(e) => {
                                let _ = ipc_tx.send(IpcResult::LastfmAuthError(format!(
                                    "saving Last.fm config failed: {e}"
                                )));
                            }
                        }
                    });
                }
                _ => {}
            }
            return;
        }

        // ─── Subsonic setup form ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::SubsonicSetup)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        match self.subsonic.form_focus {
                            0 => self.subsonic.form_server.push(c),
                            1 => self.subsonic.form_user.push(c),
                            _ => self.subsonic.form_password.push(c),
                        }
                    }
                }
                KeyCode::Backspace => match self.subsonic.form_focus {
                    0 => {
                        self.subsonic.form_server.pop();
                    }
                    1 => {
                        self.subsonic.form_user.pop();
                    }
                    _ => {
                        self.subsonic.form_password.pop();
                    }
                },
                KeyCode::Tab => {
                    self.subsonic.form_focus = (self.subsonic.form_focus + 1) % 3;
                }
                KeyCode::Enter => {
                    let server = self.subsonic.form_server.clone();
                    let user = self.subsonic.form_user.clone();
                    let password = self.subsonic.form_password.clone();
                    if server.is_empty() || user.is_empty() {
                        self.notify_typed(
                            "Subsonic",
                            "Server URL and username are required",
                            NotificationKind::Info,
                            false,
                            NotifType::Subsonic,
                        );
                        return;
                    }
                    if self.subsonic.form_focus < 2 {
                        self.subsonic.form_focus += 1;
                    } else {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.subsonic().configure(&server, &user, &password).await;
                        });
                    }
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast feeds ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastFeeds)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(feed) = self.podcast.feeds.get(sel).cloned() {
                        self.podcast.episodes.clear();
                        self.podcast.episodes_feed_id = Some(feed.id.clone());
                        self.pickers.open(PickerId::PodcastEpisodes);
                        self.fetch_podcast_episodes(feed.id);
                    }
                }
                KeyCode::Char('a') => {
                    self.podcast.subscribe_url.clear();
                    self.pickers.open(PickerId::PodcastSubscribe);
                }
                KeyCode::Char('r') => {
                    let c = self.client.clone();
                    tokio::spawn(async move {
                        let _ = c.podcast().refresh(None).await;
                    });
                    self.podcast.feeds.clear();
                    self.podcast.feeds_pending = true;
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        match c.podcast().feeds().await {
                            Ok(f) => {
                                let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                            }
                        }
                    });
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast episodes ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastEpisodes)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let idx = self.pickers.top().map_or(0, |o| o.selected);
                    let feed_id = self.podcast.episodes_feed_id.clone();
                    if let (Some(feed_id), Some(_ep)) = (feed_id, self.podcast.episodes.get(idx)) {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.podcast().play(&feed_id, idx).await;
                        });
                    }
                }
                KeyCode::Backspace => {
                    self.podcast.episodes.clear();
                    self.podcast.episodes_feed_id = None;
                    self.pickers.close_top();
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast subscribe form ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastSubscribe)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        self.podcast.subscribe_url.push(c);
                    }
                }
                KeyCode::Backspace => {
                    self.podcast.subscribe_url.pop();
                }
                KeyCode::Enter => {
                    let url = self.podcast.subscribe_url.clone();
                    if url.is_empty() {
                        return;
                    }
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    self.pickers.close_top();
                    tokio::spawn(async move {
                        match c.podcast().add_feed(&url).await {
                            Ok(_) => {
                                let _ = ipc_tx.send(IpcResult::Notification(
                                    "Podcast".into(),
                                    format!("Subscribed to {url}"),
                                    NotificationKind::Success,
                                    NotifType::Podcast,
                                ));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("subscribe failed: {e}"));
                            }
                        }
                    });
                }
                _ => {}
            }
            return;
        }

        // ─── Radio search ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::RadioSearch)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        if let Some(top) = self.pickers.top_mut() {
                            top.query.push(c);
                        }
                        self.radio.search.clear();
                    }
                }
                KeyCode::Backspace => {
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.pop();
                    }
                    self.radio.search.clear();
                }
                KeyCode::Enter => {
                    let q = self
                        .pickers
                        .top()
                        .map_or(String::new(), |o| o.query.clone());
                    if self.radio.search_pending {
                        return;
                    }
                    if self.radio.search.is_empty() {
                        self.search_radio(q);
                    } else {
                        let sel = self.pickers.top().map_or(0, |o| o.selected);
                        if let Some(station) = self.radio.search.get(sel).cloned() {
                            let c = self.client.clone();
                            self.pickers.close_top();
                            tokio::spawn(async move {
                                let _ = c.radio().play(&station.id, &station.name).await;
                            });
                        }
                    }
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Radio top stations ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::RadioTop)) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(station) = self.radio.top.get(sel).cloned() {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.radio().play(&station.id, &station.name).await;
                        });
                    }
                }
                KeyCode::Char('r') => {
                    self.radio.top.clear();
                    self.radio.top_pending = true;
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        match c.radio().top(50).await {
                            Ok(s) => {
                                let _ = ipc_tx.send(IpcResult::RadioTop(s));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("radio top failed: {e}"));
                            }
                        }
                    });
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Radio browse ───
        // Root picker: choose Tags or Countries, then drill into the list.
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::RadioBrowse)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    self.radio.browse_kind = match sel {
                        0 => RadioBrowseKind::Tags,
                        _ => RadioBrowseKind::Countries,
                    };
                    self.radio.browse_topic.clear();
                    self.radio.browse_tags.clear();
                    self.radio.browse_countries.clear();
                    self.radio.browse_stations.clear();
                    self.pickers.open(PickerId::RadioBrowseList);
                    self.on_picker_opened(PickerId::RadioBrowseList);
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Radio browse list (tags / countries) ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::RadioBrowseList)
        ) {
            match key.code {
                KeyCode::Enter => {
                    if self.radio.browse_pending {
                        return;
                    }
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    let picked = match self.radio.browse_kind {
                        RadioBrowseKind::Tags => self
                            .radio
                            .browse_tags
                            .get(sel)
                            .map(|t| (t.name.clone(), t.station_count)),
                        RadioBrowseKind::Countries => self
                            .radio
                            .browse_countries
                            .get(sel)
                            .map(|c| (c.name.clone(), c.station_count)),
                    };
                    if let Some((topic, _count)) = picked {
                        self.radio.browse_topic = topic;
                        self.pickers.open(PickerId::RadioBrowseStations);
                        self.on_picker_opened(PickerId::RadioBrowseStations);
                    }
                }
                KeyCode::Char('r') => {
                    self.fetch_radio_browse_list();
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Radio browse stations ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::RadioBrowseStations)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(station) = self.radio.browse_stations.get(sel).cloned() {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        tokio::spawn(async move {
                            let _ = c.radio().play(&station.id, &station.name).await;
                        });
                    }
                }
                KeyCode::Char('r') => {
                    self.fetch_radio_browse_stations();
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        let top_id = self.pickers.top().map(|o| o.id);
        let is_help = top_id == Some(PickerId::Help);
        let ctrl_or_alt = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => {
                self.close_top_picker_with_cleanup();
            }
            // Uniform picker navigation: Left/Right leave
            // single-section pickers like Esc, or cycle the mode in the
            // notification settings picker.
            KeyCode::Left | KeyCode::Right => {
                let top_id = self.pickers.top().map(|o| o.id);
                if top_id == Some(PickerId::NotificationSettings) {
                    self.cycle_notification_mode(if key.code == KeyCode::Right { 1 } else { -1 });
                } else if matches!(
                    top_id,
                    Some(PickerId::Equalizer)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::PlaylistSelect)
                ) {
                    self.close_top_picker_with_cleanup();
                }
            }
            // Help picker vim motions
            KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL && is_help => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = 0;
                }
            }
            KeyCode::Char('G') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = total.saturating_sub(1);
                }
            }
            KeyCode::Char('0') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = 0;
                }
            }
            KeyCode::Char('$') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = total.saturating_sub(1);
                }
            }
            KeyCode::Char('n') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut()
                    && total > 0
                {
                    top.selected = (top.selected + 1).min(total - 1);
                }
            }
            KeyCode::Char('N') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                }
            }
            // Queue move up/down (Ctrl+K/J) must come before plain k/j
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::Queue
                    && !self.queue.cache.is_empty()
                {
                    let idx = top.selected.min(self.queue.cache.len() - 1);
                    if idx > 0 {
                        let _ = tx
                            .send(TuiCommand::QueueMove(
                                idx as u64,
                                idx.saturating_sub(1) as u64,
                            ))
                            .await;
                        self.fetch_queue().await;
                    }
                }
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::Queue
                    && !self.queue.cache.is_empty()
                {
                    let idx = top.selected.min(self.queue.cache.len() - 1);
                    if idx < self.queue.cache.len() - 1 {
                        let _ = tx
                            .send(TuiCommand::QueueMove(idx as u64, (idx + 1) as u64))
                            .await;
                        self.fetch_queue().await;
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let has_input = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::YTSearch)
                        | Some(PickerId::SearchLibrary)
                        | Some(PickerId::CommandPalette)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::SpotifySearch)
                        | Some(PickerId::SpotifyLink)
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata.field_idx > 0 {
                        self.metadata.field_idx -= 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Up {
                    // Add 'k' to the query instead of navigating
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.push('k');
                    }
                    return;
                }
                let count = self.picker_item_count();
                if let Some(top) = self.pickers.top_mut() {
                    if count > 0 && top.selected == 0 {
                        top.selected = count - 1;
                    } else {
                        top.selected = top.selected.saturating_sub(1);
                    }
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_on_navigation().await;
                self.apply_preset_preview();
                // Refresh picker preview cover for SearchLibrary when selection changes
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SearchLibrary)
                {
                    self.update_picker_preview();
                }
                // Spotify search results can carry album art too: refresh the
                // preview when the highlighted row changes.
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SpotifySearch)
                {
                    self.update_spotify_search_preview();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let has_input = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::YTSearch)
                        | Some(PickerId::SearchLibrary)
                        | Some(PickerId::CommandPalette)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::SpotifySearch)
                        | Some(PickerId::SpotifyLink)
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata.field_idx < 6 {
                        self.metadata.field_idx += 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Down {
                    // Add 'j' to the query instead of navigating
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.push('j');
                    }
                    return;
                }
                let count = self.picker_item_count();
                if let Some(top) = self.pickers.top_mut() {
                    let max = count.saturating_sub(1);
                    if count > 0 && top.selected >= max {
                        top.selected = 0;
                    } else {
                        top.selected += 1;
                    }
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_on_navigation().await;
                self.apply_preset_preview();
                // Refresh picker preview cover for SearchLibrary when selection changes
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SearchLibrary)
                {
                    self.update_picker_preview();
                }
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SpotifySearch)
                {
                    self.update_spotify_search_preview();
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                ) {
                    if let Some(track_id) = self.metadata.edit_track_id {
                        let title = self.metadata.fields[0].clone();
                        let artist = self.metadata.fields[1].clone();
                        let album = self.metadata.fields[2].clone();
                        let genre = self.metadata.fields[4].clone();
                        let year = self.metadata.fields[5].parse::<i32>().ok();
                        let track_number = self.metadata.fields[6].parse::<i32>().ok();
                        let client = self.client.clone();
                        let ipc_tx = self.ipc_tx.clone();
                        tokio::spawn(async move {
                            let patch = MetadataPatch {
                                title: Some(title),
                                artist: Some(artist),
                                album: Some(album),
                                genre: Some(genre),
                                year,
                                track_number,
                                album_id: None,
                            };
                            let _ = client.library().update_metadata(track_id, patch).await;
                            let _ = ipc_tx.send(IpcResult::Notification(
                                "Library".to_string(),
                                "Metadata saved".to_string(),
                                NotificationKind::Success,
                                NotifType::Library,
                            ));
                        });
                        self.metadata.cover = None;
                        self.metadata.cover_stateful = None;
                        self.metadata.edit_track_id = None;
                    }
                    self.pickers.close_top();
                }
            }
            KeyCode::Enter => {
                // Dispatch based on picker type
                if let Some(top) = self.pickers.top() {
                    match top.id {
                        PickerId::SpotifySearch => {
                            let not_linked = self.spotify.status.as_ref().is_none_or(|s| !s.linked);
                            if not_linked {
                                let token = self.spotify.token_input.trim().to_string();
                                if token.is_empty() {
                                    self.notify_typed(
                                        "System",
                                        "Paste a Spotify access token first",
                                        NotificationKind::Error,
                                        false,
                                        NotifType::Spotify,
                                    );
                                } else {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = c.spotify().set_token(&token).await {
                                            let _ = ipc_tx.send(IpcResult::Error(format!(
                                                "Spotify token failed: {e}"
                                            )));
                                            return;
                                        }
                                        let _ = ipc_tx.send(IpcResult::Notification(
                                            "Spotify".to_string(),
                                            "Token set. Syncing playlists…".to_string(),
                                            NotificationKind::Info,
                                            NotifType::Spotify,
                                        ));
                                        match c.spotify().sync().await {
                                            Ok(()) => {
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    "Sync complete".to_string(),
                                                    NotificationKind::Success,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify sync failed: {e}"
                                                )));
                                            }
                                        }
                                        let status = c.spotify().status().await;
                                        if let Ok(s) = status {
                                            let _ = ipc_tx.send(IpcResult::SpotifyStatus(s));
                                        }
                                    });
                                    self.spotify.token_input.clear();
                                }
                            } else if self.spotify.search_results.is_empty() {
                                self.notify_typed(
                                    "System",
                                    "Type to search your synced Spotify playlists",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::Spotify,
                                );
                            } else {
                                let idx = top.selected.min(self.spotify.search_results.len() - 1);
                                let (playlist_id, _, track) =
                                    self.spotify.search_results[idx].clone();
                                let track_index = track.index;
                                let c = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                self.pickers.close_top();
                                if playlist_id == "web" {
                                    let c2 = c.clone();
                                    let ipc_tx2 = ipc_tx.clone();
                                    let track_clone = track.clone();
                                    tokio::spawn(async move {
                                        match c2
                                            .spotify()
                                            .resolve_track(
                                                &track_clone.name,
                                                &track_clone.artists,
                                                track_clone.album.as_deref().unwrap_or(""),
                                                track_clone.uri.clone(),
                                            )
                                            .await
                                        {
                                            Ok(()) => {
                                                let _ = ipc_tx2.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    format!(
                                                        "Queued: {} - {}",
                                                        track_clone.artists, track_clone.name
                                                    ),
                                                    NotificationKind::Success,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx2.send(IpcResult::Error(format!(
                                                    "Spotify resolve failed: {e}"
                                                )));
                                            }
                                        }
                                    });
                                } else {
                                    tokio::spawn(async move {
                                        match c.spotify().resolve(&playlist_id, track_index).await {
                                            Ok(()) => {
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    format!(
                                                        "Queued: {} - {}",
                                                        track.artists, track.name
                                                    ),
                                                    NotificationKind::Success,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify resolve failed: {e}"
                                                )));
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        PickerId::SpotifyLink => {
                            // An empty entry falls back to librespot's public
                            // desktop client id so no dashboard app is needed.
                            let client_id = self.spotify.link_input.trim().to_string();
                            let client_id = if client_id.is_empty() {
                                LIBRESPOT_CLIENT_ID.to_string()
                            } else {
                                client_id
                            };
                            let port = self
                                .spotify
                                .oauth_port
                                .trim()
                                .parse::<u16>()
                                .unwrap_or(8990);
                            // Keep the picker open and show a waiting state until
                            // the daemon reports the link completed.
                            self.start_spotify_oauth(client_id, port);
                        }
                        PickerId::Queue => {
                            if !self.queue.cache.is_empty() {
                                let idx = top.selected.min(self.queue.cache.len() - 1);
                                let path = self.queue.cache[idx].path.clone();
                                // Immediately update queue_cursor so the up-next preview
                                // reflects the newly selected track's next track.
                                self.queue.cursor = idx;
                                self.send_high(TuiCommand::Play(path));
                            }
                        }
                        PickerId::YTSearch => {
                            if top.query.is_empty() {
                                // Start search
                            } else if !self.yt_results_cache.is_empty() {
                                let idx = top.selected.min(self.yt_results_cache.len() - 1);
                                if self.yt_results_cache[idx].is_playlist {
                                    // Playlist drill-down: search using the playlist URL
                                    let url = self.yt_results_cache[idx].url.clone();
                                    if let Some(top) = self.pickers.top_mut() {
                                        top.query = url;
                                    }
                                    let query = self.yt_results_cache[idx].url.clone();
                                    let _ = tx.send(TuiCommand::YtSearch(query)).await;
                                } else {
                                    let url = self.yt_results_cache[idx].url.clone();
                                    let _ = tx.send(TuiCommand::YtResolve(url)).await;
                                }
                            } else {
                                // Initiate a new search
                                let query = top.query.clone();
                                let _ = tx.send(TuiCommand::YtSearch(query)).await;
                                let _ = tx.send(TuiCommand::RefreshYt).await;
                            }
                        }
                        PickerId::SleepTimer => {
                            // Handled by early return above
                        }
                        PickerId::Crossfade => {
                            let sel = top.selected;
                            // Rows: [0] "Duration" header, [1..=5] durations.
                            if (1..=5).contains(&sel) {
                                let dur = CROSSFADE_DURATIONS[sel - 1];
                                let enabled = self
                                    .state
                                    .crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(true);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                                if let Some(ref mut cf) = self.state.crossfade {
                                    cf.duration_secs = dur;
                                }
                                self.pickers.close_top();
                            }
                        }
                        PickerId::VisualizerPreset => {
                            let presets = VisualizerPreset::all();
                            if let Some(top) = self.pickers.top() {
                                let idx = top.selected.min(presets.len() - 1);
                                self.visualizer.preset = presets[idx];
                                save_prefs(&self.current_prefs());
                                self.footer_notification = Some((
                                    format!("Visualizer: {}", self.visualizer.preset.name()),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                            self.pickers.close_top();
                        }
                        PickerId::ProgressStyle => {
                            let styles = ProgressStyle::all();
                            if let Some(top) = self.pickers.top() {
                                let idx = top.selected.min(styles.len() - 1);
                                self.progress_style = styles[idx];
                                save_prefs(&self.current_prefs());
                                self.notify_typed(
                                    "System",
                                    format!("Progress: {}", self.progress_style.name()),
                                    NotificationKind::Info,
                                    false,
                                    NotifType::Playback,
                                );
                            }
                            self.pickers.close_top();
                        }
                        PickerId::FooterPreset => {
                            if let Some(top) = self.pickers.top() {
                                let idx = top
                                    .selected
                                    .min(self.footer_presets.len().saturating_sub(1));
                                self.apply_footer_preset_index(idx);
                                let name = self
                                    .footer_presets
                                    .get(self.footer_preset)
                                    .map(|p| p.name.to_string())
                                    .unwrap_or_else(|| "Default".into());
                                self.notify_typed(
                                    "System",
                                    format!("Footer preset: {name}"),
                                    NotificationKind::Info,
                                    false,
                                    NotifType::Playback,
                                );
                            }
                            self.pickers.close_top();
                        }
                        PickerId::NotificationSettings => {
                            self.cycle_notification_mode(1);
                        }
                        PickerId::CommandPalette => {
                            let commands = CommandPalette::commands(&self.icon_style);
                            let query = top.query.to_lowercase();
                            let filtered: Vec<&Command> = if query.is_empty() {
                                commands.iter().collect()
                            } else {
                                commands
                                    .iter()
                                    .filter(|c| !(c.keys.is_empty() && c.hint.is_empty()))
                                    .filter(|c| {
                                        let lower = c.icon.to_lowercase();
                                        let mut qi = 0usize;
                                        for ch in lower.chars() {
                                            if qi < query.len()
                                                && ch == query.as_bytes()[qi] as char
                                            {
                                                qi += 1;
                                            }
                                        }
                                        qi == query.len()
                                    })
                                    .collect()
                            };
                            let idx = top.selected.min(filtered.len().saturating_sub(1));
                            if let Some(cmd) = filtered.get(idx) {
                                // Dispatch on the stable action id, never on the
                                // display label, so the highlighted row's command
                                // is what actually runs.
                                let action = cmd.hint;
                                if action == "play/pause" {
                                    self.send_high(TuiCommand::PlayPause);
                                } else if action == "next track" {
                                    self.send_high(TuiCommand::Next);
                                } else if action == "prev track" {
                                    self.send_high(TuiCommand::Prev);
                                } else if action == "volume up" {
                                    let new_vol = (self.state.volume + 5).min(MAX_VOLUME);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if action == "volume down" {
                                    let new_vol = self.state.volume.saturating_sub(5);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if action == "mute" {
                                    self.send_high(TuiCommand::ToggleMute);
                                } else if action == "repeat" {
                                    let new_mode = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    self.send_high(TuiCommand::CycleRepeat(new_mode));
                                } else if action == "shuffle" {
                                    self.send_high(TuiCommand::ToggleShuffle);
                                } else if action == "quit daemon" {
                                    let c = self.client.clone();
                                    let _ =
                                        tokio::time::timeout(Duration::from_millis(1500), c.quit())
                                            .await;
                                    self.pending_quit = true;
                                } else if action == "quit" {
                                    self.pending_quit = true;
                                } else if action == "tab cycle" {
                                    self.cycle_pane_focus(true);
                                    self.pickers.close_top();
                                } else if action == "settings" {
                                    self.pickers.open(PickerId::Settings);
                                } else if action == "queue" {
                                    self.pickers.open(PickerId::Queue);
                                } else if action == "youtube" {
                                    self.pickers.open(PickerId::YTSearch);
                                } else if action == "search lib" {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if action == "eq" {
                                    self.pickers.open(PickerId::Equalizer);
                                } else if action == "sleeptimer" {
                                    self.pickers.open(PickerId::SleepTimer);
                                } else if action == "themepicker" {
                                    self.pickers.open_with_selection(
                                        PickerId::ThemePicker,
                                        self.theme_index,
                                    );
                                } else if action == "about" {
                                    self.pickers.open(PickerId::About);
                                } else if action == "notifications" {
                                    self.pickers.open(PickerId::Notifications);
                                } else if action == "search" {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if action == "spotify" {
                                    self.pickers.open(PickerId::SpotifySearch);
                                } else if action == "fetch lyrics" {
                                    self.lyrics.show = true;
                                    self.send_high(TuiCommand::FetchLyrics);
                                } else if action == "progress style" {
                                    self.pickers.open(PickerId::ProgressStyle);
                                } else if action == "visualizer preset" {
                                    self.pickers.open(PickerId::VisualizerPreset);
                                } else if action == "visualizer" {
                                    self.visualizer.toggle();
                                    let state = if self.visualizer.is_enabled() {
                                        "ON"
                                    } else {
                                        "OFF"
                                    };
                                    self.notify_typed(
                                        "System",
                                        format!("Visualizer: {}", state),
                                        NotificationKind::Info,
                                        false,
                                        NotifType::Playback,
                                    );
                                } else if action == "stop" {
                                    self.send_high(TuiCommand::Stop);
                                } else if action == "seek forward" {
                                    let pos =
                                        (self.display_position + 5.0).min(self.state.duration);
                                    self.send_high(TuiCommand::Seek(pos));
                                } else if action == "seek backward" {
                                    let pos = (self.display_position - 5.0).max(0.0);
                                    self.send_high(TuiCommand::Seek(pos));
                                } else if action == "toggle favourite" {
                                    if let Some(ref track) = self.state.current_track {
                                        let track_id = track.id;
                                        let new_fav = !track.favourite;
                                        if let Some(ref mut ct) = self.state.current_track {
                                            ct.favourite = new_fav;
                                        }
                                        for t in &mut self.tracks_cache {
                                            if t.id == track_id {
                                                t.favourite = new_fav;
                                                break;
                                            }
                                        }
                                        let tx = self.cmd_tx();
                                        let _ = tx.send(TuiCommand::AddFavourite(track_id)).await;
                                        self.notify_titled(
                                            "Library",
                                            "Favourite toggled",
                                            NotificationKind::Info,
                                            true,
                                            NotifType::Playback,
                                        );
                                    }
                                } else if action == "clear queue" {
                                    let tx = self.cmd_tx();
                                    let _ = tx.send(TuiCommand::QueueClear).await;
                                    self.footer_notification = Some((
                                        "Queue cleared".to_string(),
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(2),
                                    ));
                                } else if action == "prev tab" {
                                    self.cycle_pane_focus(false);
                                    self.pickers.close_top();
                                } else if action == "multiselect" {
                                    if !self.library_pane_focus {
                                        self.multiselect_mode = !self.multiselect_mode;
                                        if !self.multiselect_mode {
                                            self.selected_indices.clear();
                                        }
                                        let msg = if self.multiselect_mode {
                                            "Multiselect ON: use v/a/x to queue"
                                        } else {
                                            "Multiselect OFF"
                                        };
                                        self.footer_notification = Some((
                                            msg.to_string(),
                                            std::time::Instant::now()
                                                + std::time::Duration::from_secs(2),
                                        ));
                                    }
                                } else if action == "add to queue" {
                                    if !self.library_pane_focus {
                                        let tracks = self.filtered_tracks();
                                        let indices: Vec<usize> = if self.multiselect_mode
                                            && !self.selected_indices.is_empty()
                                        {
                                            self.selected_indices.iter().copied().collect()
                                        } else {
                                            vec![self.list_pos()]
                                        };
                                        let mut added = 0;
                                        for idx in indices {
                                            if let Some(track) = tracks.get(idx) {
                                                let c = self.client.clone();
                                                let path = track.path.clone();
                                                tokio::spawn(async move {
                                                    let _ = c.queue().add(&path, None).await;
                                                });
                                                added += 1;
                                            }
                                        }
                                        self.fetch_queue().await;
                                        self.notify_typed(
                                            "System",
                                            format!("Added {added} track(s) to queue"),
                                            NotificationKind::Info,
                                            false,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                } else if action == "add to playlist" {
                                    if !self.library_pane_focus {
                                        let tracks = self.filtered_tracks();
                                        let indices: Vec<i64> = if self.multiselect_mode
                                            && !self.selected_indices.is_empty()
                                        {
                                            self.selected_indices
                                                .iter()
                                                .filter_map(|i| tracks.get(*i).map(|t| t.id))
                                                .collect()
                                        } else {
                                            tracks
                                                .get(self.list_pos())
                                                .map(|t| vec![t.id])
                                                .unwrap_or_default()
                                        };
                                        if !indices.is_empty() {
                                            self.pending_playlist_track_ids = indices;
                                            self.playlist_creating = false;
                                            self.pickers.open(PickerId::PlaylistSelect);
                                        }
                                    }
                                } else if action == "delete from list" {
                                    if !self.library_pane_focus {
                                        if self.library_category == 4
                                            && self.browse_detail.is_some()
                                        {
                                            let filtered = self.filtered_tracks();
                                            if let Some(track) = filtered.get(self.list_pos()) {
                                                let track_id = track.id;
                                                if let Some(pl) =
                                                    self.playlist_cache.iter().find(|p| {
                                                        self.browse_detail.as_deref()
                                                            == Some(&p.name)
                                                    })
                                                {
                                                    let playlist_id = pl.id;
                                                    let tx = self.cmd_tx();
                                                    let _ = tx
                                                        .send(TuiCommand::RemoveFromPlaylist(
                                                            playlist_id,
                                                            track_id,
                                                        ))
                                                        .await;
                                                    self.notify_typed(
                                                        "System",
                                                        "Removed from playlist",
                                                        NotificationKind::Info,
                                                        false,
                                                        NotifType::NowPlaying,
                                                    );
                                                }
                                            }
                                        } else {
                                            self.notify_typed(
                                                "System",
                                                "Remove from list only available in playlist view",
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        }
                                    }
                                } else if action == "jump to end" {
                                    if !self.library_pane_focus {
                                        let max = self.library_list_len().saturating_sub(1);
                                        self.set_list_pos(max);
                                    }
                                } else if action == "edit metadata" {
                                    if !self.library_pane_focus {
                                        let track_data = {
                                            let tracks = self.filtered_tracks();
                                            tracks.get(self.list_pos()).map(|t| {
                                                (
                                                    t.id,
                                                    t.title.clone(),
                                                    t.artist.clone(),
                                                    t.album.clone(),
                                                    t.genre.clone(),
                                                    t.year,
                                                    t.track_number,
                                                )
                                            })
                                        };
                                        if let Some((
                                            id,
                                            title,
                                            artist,
                                            album,
                                            genre,
                                            year,
                                            track_num,
                                        )) = track_data
                                        {
                                            self.metadata.edit_track_id = Some(id);
                                            self.metadata.fields = [
                                                title,
                                                artist,
                                                album,
                                                String::new(),
                                                genre,
                                                year.map_or(String::new(), |y| y.to_string()),
                                                track_num.map_or(String::new(), |n| n.to_string()),
                                            ];
                                            self.metadata.field_idx = 0;
                                            self.pickers.open(PickerId::EditMetadata);
                                            self.fetch_metadata_cover();
                                        }
                                    }
                                } else if action == "toggle help" {
                                    if self.pickers.top().is_some_and(|o| o.id == PickerId::Help) {
                                        self.pickers.close_top();
                                    } else {
                                        self.pickers.open(PickerId::Help);
                                    }
                                } else if action == "hide help bar" {
                                    self.hide_help_bar = !self.hide_help_bar;
                                } else if action == "health check" {
                                    self.send_high(TuiCommand::CheckHealth);
                                } else if action == "setup" {
                                    self.open_setup_picker(None);
                                } else if action == "radio browse" {
                                    self.pickers.open(PickerId::RadioBrowse);
                                    self.on_picker_opened(PickerId::RadioBrowse);
                                }
                            }
                            // If the action opened a sub-picker it was stacked on
                            // top of the palette; leave it open.  Otherwise the
                            // palette closes.
                            if self
                                .pickers
                                .top()
                                .is_some_and(|o| o.id == PickerId::CommandPalette)
                            {
                                self.pickers.close_top();
                            }
                        }
                        PickerId::PlaylistSelect => {
                            if self.playlist_creating {
                                let name = top.query.trim().to_string();
                                if !name.is_empty() {
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        match client.library().create_playlist(&name).await {
                                            Ok(playlists) => {
                                                if let Some(new_p) = playlists.first().cloned() {
                                                    let playlists =
                                                        client.library().get_playlists().await;
                                                    if let Ok(DaemonRes::Playlists {
                                                        playlists,
                                                        ..
                                                    }) = playlists
                                                    {
                                                        let _ = ipc_tx
                                                            .send(IpcResult::Playlists(playlists));
                                                    }
                                                    let _ =
                                                        ipc_tx.send(IpcResult::PlaylistCreated(
                                                            new_p.id,
                                                            name.clone(),
                                                        ));
                                                    let _ = ipc_tx.send(IpcResult::Notification(
                                                        "Playlist".to_string(),
                                                        format!("Created {name} — pick tracks"),
                                                        NotificationKind::Success,
                                                        NotifType::NowPlaying,
                                                    ));
                                                }
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Failed to create playlist: {e}"
                                                )));
                                            }
                                        }
                                    });
                                    self.playlist_creating = false;
                                    self.close_top_picker_with_cleanup();
                                }
                            } else if top.selected == 0 {
                                self.playlist_creating = true;
                            } else {
                                let idx = top.selected.saturating_sub(1);
                                let track_ids = self.pending_playlist_track_ids.clone();
                                if let Some(pl) = self.playlist_cache.get(idx) {
                                    let playlist_id = pl.id;
                                    if !track_ids.is_empty() {
                                        let client = self.client.clone();
                                        tokio::spawn(async move {
                                            let _ = client
                                                .library()
                                                .add_to_playlist(playlist_id, track_ids)
                                                .await;
                                        });
                                        self.notify_titled(
                                            "Playlist",
                                            "Added to playlist",
                                            NotificationKind::Success,
                                            true,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                    self.close_top_picker_with_cleanup();
                                }
                            }
                        }
                        PickerId::Equalizer => {
                            // Apply selected EQ preset
                            let idx = top.selected.min(EQ_PRESETS.len() - 1);
                            let c = self.client.clone();
                            let preset = EQ_PRESETS[idx];
                            tokio::spawn(async move {
                                let _ = c.set_eq_preset(preset).await;
                            });
                            self.footer_notification = Some((
                                format!("Equalizer preset: {}", preset.label()),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                            self.pickers.close_top();
                        }
                        PickerId::ThemePicker => {
                            let idx = top.selected;
                            self.apply_theme_index(idx);
                            let name = &self.themes[idx].name;
                            let light = if self.themes[idx].light {
                                " (light)"
                            } else {
                                ""
                            };
                            self.notify_titled(
                                "Theme",
                                format!("Theme: {}{}", name, light),
                                NotificationKind::Info,
                                true,
                                NotifType::Prefs,
                            );
                            self.pickers.close_top();
                        }
                        PickerId::SearchLibrary => {
                            let picks = self.search_library_picks();
                            if !picks.is_empty() {
                                let idx = top.selected.min(picks.len() - 1);
                                match &picks[idx] {
                                    LibraryPick::Track(i) => {
                                        let path = self.tracks_cache[*i].path.clone();
                                        self.send_high(TuiCommand::Play(path));
                                    }
                                    LibraryPick::Artist(name) => {
                                        self.reset_library_view(3, Some(name.clone()));
                                        self.library_pane_focus = false;
                                    }
                                    LibraryPick::Album(album) => {
                                        self.reset_library_view(2, Some(album.clone()));
                                        self.library_pane_focus = false;
                                    }
                                    LibraryPick::Playlist(i) => {
                                        let playlist = self.playlist_cache[*i].clone();
                                        self.reset_library_view(4, Some(playlist.name.clone()));
                                        self.library_pane_focus = false;
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let pid = playlist.id;
                                        tokio::spawn(async move {
                                            if let Ok(DaemonRes::Tracks { tracks }) =
                                                c.library().get_playlist_tracks(pid).await
                                            {
                                                let _ =
                                                    ipc_tx2.send(IpcResult::PlaylistTracks(tracks));
                                            }
                                        });
                                    }
                                }
                            }
                            self.pickers.close_top();
                        }
                        PickerId::EditMetadata => {
                            if self.metadata.field_idx < 6 {
                                self.metadata.field_idx += 1;
                            } else {
                                if let Some(track_id) = self.metadata.edit_track_id {
                                    let title = self.metadata.fields[0].clone();
                                    let artist = self.metadata.fields[1].clone();
                                    let album = self.metadata.fields[2].clone();
                                    let genre = self.metadata.fields[4].clone();
                                    let year = self.metadata.fields[5].parse::<i32>().ok();
                                    let track_number = self.metadata.fields[6].parse::<i32>().ok();
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        let patch = MetadataPatch {
                                            title: Some(title),
                                            artist: Some(artist),
                                            album: Some(album),
                                            genre: Some(genre),
                                            year,
                                            track_number,
                                            album_id: None,
                                        };
                                        let _ =
                                            client.library().update_metadata(track_id, patch).await;
                                        let _ = ipc_tx.send(IpcResult::Notification(
                                            "Library".to_string(),
                                            "Metadata saved".to_string(),
                                            NotificationKind::Success,
                                            NotifType::Library,
                                        ));
                                    });
                                    self.metadata.cover = None;
                                    self.metadata.cover_stateful = None;
                                    self.metadata.edit_track_id = None;
                                }
                                self.pickers.close_top();
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: download selected result
                let idx = self.pickers.top().map_or(0, |o| {
                    o.selected
                        .min(self.yt_results_cache.len().saturating_sub(1))
                });
                if !self.yt_results_cache.is_empty() {
                    let result = &self.yt_results_cache[idx];
                    let url = result.url.clone();
                    let title = Some(result.title.clone());
                    let artist = result
                        .artist
                        .clone()
                        .or_else(|| Some(result.channel.clone()));
                    let _ = tx.send(TuiCommand::YtDownload { url, title, artist }).await;
                }
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: add selected result to queue
                let idx = self.pickers.top().map_or(0, |o| {
                    o.selected
                        .min(self.yt_results_cache.len().saturating_sub(1))
                });
                if !self.yt_results_cache.is_empty() {
                    let url = self.yt_results_cache[idx].url.clone();
                    if self.yt_results_cache[idx].is_playlist {
                        let _ = tx.send(TuiCommand::YtResolve(url)).await;
                    } else {
                        let _ = tx.send(TuiCommand::QueueAdd(url)).await;
                    }
                }
            }
            KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                // Edit Metadata: sync cover using the currently-entered metadata.
                if top_id == Some(PickerId::EditMetadata)
                    && let Some(track_id) = self.metadata.edit_track_id
                {
                    let title = self.metadata.fields[0].clone();
                    let artist = self.metadata.fields[1].clone();
                    let album = self.metadata.fields[2].clone();
                    let genre = self.metadata.fields[4].clone();
                    let year = self.metadata.fields[5].parse::<i32>().ok();
                    let track_number = self.metadata.fields[6].parse::<i32>().ok();
                    let fetch_gen = self.next_cover_gen();
                    self.metadata.cover_fetch_gen = Some(fetch_gen);
                    let client = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        let patch = MetadataPatch {
                            title: Some(title),
                            artist: Some(artist),
                            album: Some(album),
                            genre: Some(genre),
                            year,
                            track_number,
                            album_id: None,
                        };
                        // Persist edited metadata first so the cover lookup
                        // uses the updated artist/album/title.
                        let _ = client.library().update_metadata(track_id, patch).await;
                        if let Ok(Some(b64)) = client.art().cover(track_id).await
                            && let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(&b64)
                        {
                            let _ = ipc_tx.send(IpcResult::MetadataCoverArt(
                                Some(bytes),
                                track_id,
                                fetch_gen,
                            ));
                        }
                    });
                    self.metadata.cover_dirty = true;
                    self.notify_titled(
                        "Library",
                        "Syncing cover…",
                        NotificationKind::Info,
                        true,
                        NotifType::Library,
                    );
                }
            }
            KeyCode::Char(c) if !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::YTSearch
                        | PickerId::SearchLibrary
                        | PickerId::CommandPalette
                        | PickerId::ThemePicker => {
                            top.query.push(c);
                            if top.id == PickerId::YTSearch {
                                // Invalidate stale results immediately so the
                                // picker never shows results from an older query.
                                self.yt_results_cache.clear();
                                self.yt_search_loading = false;
                                self.yt_search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        PickerId::EditMetadata => {
                            self.metadata.fields[self.metadata.field_idx].push(c);
                        }
                        PickerId::SpotifySearch => {
                            if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                                self.spotify.token_input.push(c);
                            } else {
                                top.query.push(c);
                                // Invalidate stale results immediately and
                                // re-search after the debounce elapses.
                                self.spotify.search_results.clear();
                                self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                                self.spotify.search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        PickerId::SpotifyLink => {
                            if self.spotify.link_field == 0 {
                                self.spotify.link_input.push(c);
                            } else {
                                self.spotify.oauth_port.push(c);
                            }
                        }
                        PickerId::PlaylistSelect if self.playlist_creating => {
                            top.query.push(c);
                        }
                        PickerId::PlaylistSelect if c == 'n' || c == 'N' => {
                            top.query.clear();
                            self.playlist_creating = true;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(top) = self.pickers.top_mut() {
                    if top.id == PickerId::SearchLibrary {
                        top.source = top.source.next();
                        top.selected = 0;
                        top.viewport_offset = 0;
                        self.picker_preview_cover = None;
                        self.picker_preview_stateful = None;
                        self.last_picker_preview_fetch_id = None;
                        self.artist_cover = None;
                        self.artist_cover_stateful = None;
                        self.last_artist_cover_fetch = None;
                    } else if top.id == PickerId::EditMetadata {
                        self.metadata.field_idx = (self.metadata.field_idx + 1) % 7;
                    } else if top.id == PickerId::SpotifyLink {
                        self.spotify.link_field = (self.spotify.link_field + 1) % 2;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::EditMetadata => {
                            self.metadata.fields[self.metadata.field_idx].pop();
                        }
                        PickerId::SpotifySearch => {
                            if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                                self.spotify.token_input.pop();
                            } else {
                                top.query.pop();
                                self.spotify.search_results.clear();
                                self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                                self.spotify.search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        PickerId::SpotifyLink => {
                            if self.spotify.link_field == 0 {
                                self.spotify.link_input.pop();
                            } else {
                                self.spotify.oauth_port.pop();
                            }
                        }
                        PickerId::PlaylistSelect if self.playlist_creating => {
                            top.query.pop();
                        }
                        _ => {
                            top.query.pop();
                            if top.id == PickerId::YTSearch {
                                self.yt_results_cache.clear();
                                self.yt_search_loading = false;
                                self.yt_search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_paste(&mut self, text: &str) {
        if let Some(top) = self.pickers.top_mut() {
            match top.id {
                PickerId::SpotifySearch => {
                    if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                        self.spotify.token_input.push_str(text);
                    } else {
                        top.query.push_str(text);
                        self.spotify.search_results.clear();
                        self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                        self.spotify.search_debounce =
                            Some(std::time::Instant::now() + Duration::from_millis(500));
                    }
                }
                PickerId::SpotifyLink => {
                    if self.spotify.link_field == 0 {
                        self.spotify.link_input.push_str(text);
                    } else {
                        self.spotify.oauth_port.push_str(text);
                    }
                }
                PickerId::EditMetadata => {
                    self.metadata.fields[self.metadata.field_idx].push_str(text);
                }
                PickerId::PlaylistSelect if self.playlist_creating => {
                    top.query.push_str(text);
                }
                PickerId::YTSearch
                | PickerId::SearchLibrary
                | PickerId::CommandPalette
                | PickerId::ThemePicker => {
                    top.query.push_str(text);
                    if top.id == PickerId::YTSearch {
                        self.yt_results_cache.clear();
                        self.yt_search_loading = false;
                        self.yt_search_debounce =
                            Some(std::time::Instant::now() + Duration::from_millis(500));
                    }
                }
                _ => {}
            }
        }
    }

    async fn apply_eq_on_navigation(&mut self) {
        if let Some(top) = self.pickers.top()
            && top.id == PickerId::Equalizer
        {
            let idx = top.selected.min(EQ_PRESETS.len() - 1);
            self.send_high(TuiCommand::SetEqPreset(EQ_PRESETS[idx]));
            self.state.audio.eq_preset = EQ_PRESETS[idx];
        }
    }

    fn apply_preset_preview(&mut self) {
        if let Some(top) = self.pickers.top() {
            match top.id {
                PickerId::VisualizerPreset => {
                    let presets = VisualizerPreset::all();
                    let idx = top.selected.min(presets.len() - 1);
                    self.visualizer.preset = presets[idx];
                }
                PickerId::ProgressStyle => {
                    let styles = ProgressStyle::all();
                    let idx = top.selected.min(styles.len() - 1);
                    self.progress_style = styles[idx];
                }
                PickerId::FooterPreset => {
                    let idx = top
                        .selected
                        .min(self.footer_presets.len().saturating_sub(1));
                    self.footer_preset = idx;
                    self.footer_cache.suppress_refresh = false;
                }
                _ => {}
            }
        }
    }

    /// Interleave YT search results: insert one playlist entry after every 3 track entries.
    fn interleave_yt_results(mut results: Vec<YTSearchResult>) -> Vec<YTSearchResult> {
        let tracks: Vec<_> = results.drain(..).filter(|r| !r.is_playlist).collect();
        let playlists: Vec<_> = results; // remaining are playlists
        let mut out = Vec::with_capacity(tracks.len() + playlists.len());
        let mut pl_idx = 0;
        for (i, track) in tracks.into_iter().enumerate() {
            out.push(track);
            if (i + 1) % 3 == 0 && pl_idx < playlists.len() {
                out.push(playlists[pl_idx].clone());
                pl_idx += 1;
            }
        }
        // Append remaining playlists
        while pl_idx < playlists.len() {
            out.push(playlists[pl_idx].clone());
            pl_idx += 1;
        }
        out
    }
}

/// Index of the active time-synced lyric line for a playback position.
/// Untimed lines (timestamp < 0) are skipped for matching but keep their
/// index so the highlight tracks timed lines correctly. Uses
/// rposition semantics over sorted timed entries.
fn lyric_index_at(lines: &[LrcLine], position: f64) -> usize {
    if lines.is_empty() {
        return 0;
    }
    // Last timed line with timestamp <= position. Untimed
    // lines keep their index but never match; before the first timestamp
    // (and for plain lyrics) the highlight rests on line 0.
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.timestamp >= 0.0 && l.timestamp <= position)
        .map(|(i, _)| i)
        .next_back()
        .unwrap_or(0)
}

/// Whether the current lyrics have any time-synced lines. Plain lyrics
/// (`timestamp < 0` for all lines) should not highlight an active line.
pub fn lyrics_are_synced(lines: &[LrcLine]) -> bool {
    lines.iter().any(|l| l.timestamp >= 0.0)
}

/// Pure focus-state transition for Tab/Shift-Tab pane cycling on the Library
/// tab with lyrics open.  States are `(library_focus, lyrics_focus)`:
/// left `(true, false)`, right `(false, false)`, lyrics `(false, true)`.
/// Returns the next `(library_focus, lyrics_focus)` moving forward (Tab) or
/// backward (Shift-Tab) around the three-pane cycle.
fn cycle_library_focus(library_focus: bool, lyrics_focus: bool, forward: bool) -> (bool, bool) {
    if lyrics_focus {
        // lyrics → left (Tab) or right (Shift-Tab)
        (forward, false)
    } else if library_focus {
        // left → right (Tab) or lyrics (Shift-Tab)
        if forward {
            (false, false)
        } else {
            (false, true)
        }
    } else if forward {
        // right → lyrics
        (false, true)
    } else {
        // right → left
        (true, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_index_tracks_position_through_timed_lines() {
        let lines = vec![
            LrcLine {
                timestamp: -1.0,
                text: "intro (untimed)".into(),
            },
            LrcLine {
                timestamp: 0.0,
                text: "first".into(),
            },
            LrcLine {
                timestamp: 5.0,
                text: "second".into(),
            },
            LrcLine {
                timestamp: 10.0,
                text: "third".into(),
            },
        ];
        assert_eq!(lyric_index_at(&lines, -1.0), 0);
        assert_eq!(lyric_index_at(&lines, 0.0), 1);
        assert_eq!(lyric_index_at(&lines, 4.9), 1);
        assert_eq!(lyric_index_at(&lines, 5.0), 2);
        assert_eq!(lyric_index_at(&lines, 999.0), 3);
    }

    #[test]
    fn lyric_index_empty_returns_zero() {
        assert_eq!(lyric_index_at(&[], 42.0), 0);
    }

    #[test]
    fn cycle_library_focus_advances_forward() {
        let (lib, lyr) = cycle_library_focus(true, false, true);
        assert_eq!((lib, lyr), (false, false));
        let (lib, lyr) = cycle_library_focus(false, false, true);
        assert_eq!((lib, lyr), (false, true));
        let (lib, lyr) = cycle_library_focus(false, true, true);
        assert_eq!((lib, lyr), (true, false));
    }

    #[test]
    fn cycle_library_focus_advances_backward() {
        let (lib, lyr) = cycle_library_focus(true, false, false);
        assert_eq!((lib, lyr), (false, true));
        let (lib, lyr) = cycle_library_focus(false, false, false);
        assert_eq!((lib, lyr), (true, false));
        let (lib, lyr) = cycle_library_focus(false, true, false);
        assert_eq!((lib, lyr), (false, false));
    }
}
