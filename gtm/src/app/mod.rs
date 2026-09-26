// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Application state machine: input handling, IPC dispatch, crossfade
//
// This is free software released under the GPL-3.0 license.
// The imports below are this module's single shared import set; each
// submodule reaches them with one `use crate::app::*;`, so a few are
// unused here by design.
#![allow(unused_imports)]
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use crate::shared::client::{DaemonClient, LastfmStatus};
use crate::shared::custom::CustomRadioStation;
use crate::shared::global::{DaemonState, EqPreset, PlaybackStatus, RepeatMode};
use crate::shared::ipc::{CacheKind, DaemonEvent, DaemonRes, HealthReport, SyncKind};
use crate::shared::log::log;
use crate::shared::podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};
use crate::shared::radio::{RadioCountry, RadioStation, RadioTag, RadioTrack};
use crate::shared::secret::{SPOTIFY_CLIENT_ID, get_secret, set_secret};
use crate::shared::spotify::{
    LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifySearchKind, SpotifyStatus, SpotifyTrack,
};
use crate::shared::state::{ThemeMode, TrackSort, path_is_remote};
use crate::shared::track::{LrcData, LrcLine, Playlist, TrackInfo, YTSearchResult};
use crate::shared::{CoreError, MAX_SPEED, MAX_VOLUME, MIN_SPEED, MetadataPatch};
use crossterm::event::{
    self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::layout::Alignment;
use ratatui::widgets::Paragraph;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tachyonfx::EffectManager;
use tokio::sync::mpsc;

use base64::Engine;

use crate::extensions::{ExtensionId, ExtensionsConfig};
use crate::footer::{FooterCache, FooterKeyAction, FooterPreset, is_live_stream, merged_presets};
use crate::keymap::{
    BoundCommand, KeyContext, Keybindings, KeyboardAction, default_keybindings, detect_clashes,
    format_key_event, parse_key_event,
};
use crate::mouse::{MouseMap, MouseZone};
use crate::oauth::{lastfm_callback_port, open_browser};
use crate::picker::{PickerId, PickerManager, PickerSource};
use crate::progress::{ProgressSmoother, ProgressStyle};
use crate::reactive::{ReactivePalette, derive_theme, extract_palette};
use crate::theme::{AppTheme, ThemeEntry, blend_colors, chadrula, detect_os_theme, merged_themes};
use crate::ui;
use crate::ui::{
    CROSSFADE_DURATIONS, Command, CommandPalette, HELP_LINES, cover_provider_label,
    theme_mode_label, use_nerd_fonts,
};
use crate::visualizer::{AudioVisualizer, VisualizerPreset};
pub const NUM_SETTINGS_CATEGORIES: usize = 4;
pub const LIBRARY_CATEGORIES: &[&str] = &[
    "All Tracks",
    "Liked",
    "Albums",
    "Artists",
    "Playlists",
    "Spotify",
    "Radio",
    "Most Played",
    "Recently Played",
    "Recently Added",
    "Genres",
    "Folders",
    "Top Charts",
];
/// Sanitize a TOML `left_pane_lists` value: keep only canonical category
/// names, drop duplicates, preserve user order. Empty (or fully unknown)
/// input falls back to the full default set so the pane always renders.
pub fn sanitize_left_pane_lists(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if LIBRARY_CATEGORIES.contains(&n.as_str()) && !out.iter().any(|e| e == n) {
            out.push(n.clone());
        }
    }
    if out.is_empty() {
        default_left_pane_lists()
    } else {
        out
    }
}

pub fn folder_dir(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown Folder".into())
}

pub fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown Folder".into())
}
/// Returns true if the terminal doesn't support image protocols (Neovim, Zellij, etc.).
pub fn no_image_protocol() -> bool {
    std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok()
}
/// Generation-keyed cache for one `unique_*` category scan: the cached
/// rows are valid only for the `tracks_cache_gen` that produced them.
type CachedCategories = std::sync::Mutex<Option<(u64, Vec<(String, usize)>)>>;
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
    pub(crate) raw_position: f64,
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
    /// Generation bumped on every wholesale `tracks_cache` replacement; keys
    /// the `unique_*` caches below so per-frame renders don't rebuild maps.
    tracks_cache_gen: u64,
    cached_albums: CachedCategories,
    cached_artists: CachedCategories,
    cached_genres: CachedCategories,
    cached_folders: CachedCategories,
    pub queue: QueueView,
    pub browse_detail: Option<String>,
    pub yt_results_cache: Vec<YTSearchResult>,
    pub playlist_cache: Vec<Playlist>,
    pub most_played_cache: Vec<TrackInfo>,
    pub recently_played_cache: Vec<TrackInfo>,
    pub recently_added_cache: Vec<TrackInfo>,
    pub playlist_tracks_cache: Vec<TrackInfo>,
    pub spotify: SpotifyView,
    pub charts: ChartsView,
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
    /// Whether the footer's `KeyAction` segment shows the action name or the
    /// pressed key. Read-only from the TUI: set in `config.toml`.
    pub footer_key_action: FooterKeyAction,
    /// OS output devices offered by the audio device picker, filled on open.
    pub audio_devices: Vec<String>,
    /// Cover provider preference (`auto`/`deezer`/`musicbrainz`/`spotify`),
    /// persisted in config.toml and consumed by the daemon for cover lookups.
    pub cover_provider: String,
    /// On-disk cover cache budget in MiB, pushed to the daemon on change.
    pub cover_cache_mb: u64,
    /// Last reported cover cache disk usage in bytes.
    pub cover_cache_bytes: u64,
    /// About window: decorative visualization state (preset rotates, waveform
    /// is synthetic so it animates even when nothing is playing).
    pub about_viz: AboutViz,
    /// Whether to automatically fetch lyrics on track change
    pub auto_fetch_lyrics: bool,
    /// Icon style for command palette: "mdi" (Material Design Icons) or "emoji"
    pub icon_style: String,
    pub crossfade_duration: u8,
    pub yt_search_loading: bool,
    pub yt_search_debounce: Option<std::time::Instant>,
    pub search_deadline: Option<std::time::Instant>,
    /// Live download progress (keyed by daemon download id), surfaced in the
    /// footer Download module.
    pub downloads: std::collections::HashMap<u64, DownloadProgressView>,
    /// URLs with a download in flight. Kept so the YT search list can render
    /// "Download started" inline on the row (the toast is only for the
    /// finished event). Cleared when a terminal status arrives.
    pub downloading_urls: std::collections::HashSet<String>,
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
    pri_cmd_rx: mpsc::UnboundedReceiver<TuiCommand>,
    pri_cmd_tx: mpsc::UnboundedSender<TuiCommand>,
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
    path_display: Option<String>,
    prev_track_id: Option<i64>,
    prev_status: PlaybackStatus,
    prev_volume: u8,
    prev_cover_id: Option<i64>,
    cover_art_dirty: bool,
    /// Set whenever the daemon pushes events or refreshes state so the next
    /// frame re-renders even if no visual trigger (position/animation) is
    /// active yet. Cleared after each forced render.
    data_dirty: bool,
    pub footer_cache: FooterCache,
    pub footer_presets: Vec<FooterPreset>,
    pub footer_preset: usize,
    last_event_time: std::time::Instant,
    pub multiselect_mode: bool,
    pub progress_style: ProgressStyle,
    pub visualizer: AudioVisualizer,
    /// Config-driven component registry (`[extensions]` in the TUI config).
    pub extensions: ExtensionsConfig,
    /// Stable selection keys (library file path / chart URI) of the rows
    /// selected in Select mode. Every mutation and every batch operation
    /// routes through these keys, and operations re-resolve them against the
    /// current visible list at call time, so stale or shifted indices can
    /// never make a batch op act on the wrong track or silently fail.
    pub selected_keys: std::collections::HashSet<String>,
    pending_motion: Option<char>,
    pub pending_track_ids: Vec<i64>,
    /// Id of a freshly-created playlist awaiting track selection.
    pub pending_playlist_id: Option<i64>,
    /// Tracks currently highlighted for the in-flight new-playlist flow.
    pub selected_track_ids: std::collections::HashSet<i64>,
    pub playlist_creating: bool,
    /// Playlist being renamed via the PlaylistSelect name input, if any.
    pub renaming_playlist: Option<i64>,
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
    pub popup_track_id: Option<i64>,
    pub track_popup_cover: Option<Vec<u8>>,
    pub popup_cover_stateful: Option<StatefulProtocol>,
    popup_slot: FetchSlot<i64>,
    /// In-flight fetch slot for the Spotify drill-down popup cover, keyed by
    /// the track's album-image URL instead of a local library id.
    spotify_popup_slot: FetchSlot<String>,
    /// Cover art for the SearchLibrary picker preview window.
    pub picker_preview_cover: Option<Vec<u8>>,
    pub picker_preview_stateful: Option<StatefulProtocol>,
    picker_slot: FetchSlot<i64>,
    /// Cover art for artist selections in the search picker preview.
    pub artist_cover: Option<Vec<u8>>,
    pub artist_cover_stateful: Option<StatefulProtocol>,
    artist_slot: FetchSlot<String>,
    /// Active "Up Next" crossfade-countdown notification.
    pub upnext: Option<UpNextNotif>,
    // Monotonic generation counter for all cover fetches — disambiguates
    // stale responses and `id == 0` reuse across different tracks.
    next_cover_gen: u64,
    pub lyrics: LyricsView,
    /// Zen-mode flag: fullscreen cover/lyrics/visualizer surfaces.
    pub zen: bool,
    /// The active Zen-mode surface (only one is rendered at a time).
    pub zen_surface: ZenSurface,
    pub show_health_panel: bool,
    pub report_health: bool,
    pub health_report: Option<HealthReport>,
    pub hide_help_bar: bool,
    pub hide_footer: bool,
    /// Visible left-pane categories (canonical names from TOML, sanitized).
    pub left_pane_lists: Vec<String>,
    /// Master switch for the left-pane track preview card.
    pub show_preview: bool,
    pub pending_suspend: bool,
    last_config_mtime: Option<std::time::SystemTime>,
}

pub(crate) enum IpcResult {
    RefreshDone(Box<DaemonState>, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>, u64),
    PopupCoverArt(Option<Vec<u8>>, i64, u64),
    UpNextCover(Option<Vec<u8>>, i64, u64),
    QueuePreviewCover(Option<Vec<u8>>, i64, u64),
    PickerPreviewCover(Option<Vec<u8>>, i64, u64),
    MetadataCoverArt(Option<Vec<u8>>, i64, u64),
    ArtistCoverArt(Option<Vec<u8>>, String, u64),
    SpotifyPreviewCover(Option<Vec<u8>>, String, u64),
    /// Album cover bytes for the highlighted Spotify drill-down row, keyed by
    /// its image URL (guarded via `spotify_popup_slot`).
    SpotifyPopupCover(Option<Vec<u8>>, String, u64),
    CoverPicker(Option<Picker>),
    Lyrics(Option<LrcData>, u64),
    LibraryTracks(Vec<TrackInfo>),
    MostPlayed(Vec<TrackInfo>),
    RecentlyPlayed(Vec<TrackInfo>),
    RecentlyAdded(Vec<TrackInfo>),
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
        rate_bps: Option<f64>,
        eta_secs: Option<u64>,
    },
    Notification(String, String, NotificationKind, NotifType),
    Error(String),
    HealthReport(HealthReport),
    SpotifyStatus(SpotifyStatus),
    SpotifyPlaylists(Vec<SpotifyPlaylist>),
    /// A TUI-spawned playlist sync finished; `true` = success, `false` =
    /// failure (an `Error` result carries the reason). Clears the in-flight
    /// guard and arms the "synced once" latch on success.
    SpotifySyncFinished(bool),
    SpotifyTracks(Vec<SpotifyTrack>),
    /// Terminal outcome of a web search, tagged with the query generation that
    /// spawned it. Carries the failure so a failed search clears the spinner
    /// instead of leaving it up over an already-populated result list.
    SpotifySearchWebDone(u64, std::result::Result<Vec<SpotifyTrack>, String>),
    ReactivePalette(Option<ReactivePalette>),
    PodcastStatus(Option<PodcastStatus>),
    PodcastFeeds(Vec<PodcastFeed>),
    PodcastEpisodes(Vec<PodcastEpisode>),
    RadioSearch(Vec<RadioStation>),
    RadioTop(Vec<RadioStation>),
    RadioTags(Vec<RadioTag>),
    RadioCountries(Vec<RadioCountry>),
    RadioBrowseStations(Vec<RadioStation>),
    ChartsLoaded(Vec<crate::shared::chart::ChartPlaylist>),
    ChartTracksLoaded(Vec<crate::shared::chart::ChartTrack>),
    ChartsSources(Vec<crate::shared::chart::ChartSource>),
    /// Last.fm link status refreshed after a setup action completes.
    LastfmStatus(Option<LastfmStatus>),
    /// Authorize URL produced by a provider's OAuth flow. Kept separate from
    /// `Notification` so the link picker can render it inline.
    AuthUrl(&'static str, String),
    /// A hard failure of an OAuth flow (the daemon could not even start it).
    AuthError(&'static str, String),
    /// The browser could not be opened automatically. The authorize URL is
    /// shown inline in the picker, so this is recorded without a floating card.
    AuthFallback(String),
    /// Live cover cache disk usage in bytes, for the Settings row.
    CoverCacheStat(u64),
    /// OS audio output devices reported by the daemon, for the device picker.
    AudioDevices(Vec<String>),
}
/// Send a background-task error into the TUI event stream as an Error
/// (surfaced in the notification history).
fn self_err(ipc_tx: &mpsc::UnboundedSender<IpcResult>, msg: String) {
    let _ = ipc_tx.send(IpcResult::Error(msg));
}

/// Copy `text` to the system clipboard without adding a clipboard dependency:
/// feed it to the platform's canonical CLI (`wl-copy` on Wayland, `xclip` on
/// X11, `pbcopy`/`clip` elsewhere). The payload is written to stdin and the
/// tool detaches itself to serve the selection, so this never blocks the UI
/// loop. Best-effort: an unavailable tool surfaces as a readable error.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let tool: &str = if cfg!(target_os = "macos") {
        "pbcopy"
    } else if cfg!(target_os = "windows") {
        "clip"
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "wl-copy"
    } else if std::env::var_os("DISPLAY").is_some() {
        "xclip"
    } else {
        // No display env exported (e.g. some WSL / ssh setups): still try the
        // most common name so it works when the tool is on PATH anyway.
        "wl-copy"
    };
    let mut child = std::process::Command::new(tool)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("`{tool}` unavailable: {e}"))?;
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().ok_or("clipboard stdin unavailable")?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("write to `{tool}`: {e}"))?;
    } // stdin dropped -> EOF; wl-copy/xclip detach and serve the selection.
    Ok(())
}

/// Open the OAuth URL in a browser. When no opener works the authorize URL is
/// already rendered inline in the link picker, so only a quiet fallback notice
/// is recorded (no floating card). Non-blocking.
fn try_open_browser(url: &str, ipc_tx: &mpsc::UnboundedSender<IpcResult>) {
    let url = url.to_string();
    let ipc_tx = ipc_tx.clone();
    tokio::spawn(async move {
        if open_browser(&url).await {
            return;
        }
        // All openers failed: point at the URL shown inline in the picker.
        let _ = ipc_tx.send(IpcResult::AuthFallback(
            "Could not open a browser automatically — copy the authorize URL shown in this picker"
                .into(),
        ));
    });
}

/// Validate a typed Spotify client id before starting the PKCE flow, and
/// remind the user of the redirect-URI requirement: when the URI is missing
/// from the app dashboard the flow fails silently inside the browser (see
/// `docs/spec/spotify-linking.md`). The empty-input fallback (librespot's
/// public desktop id) always passes this check.
fn client_id_error(client_id: &str, port: u16) -> Option<String> {
    if client_id.len() != 32 || !client_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!(
            "This doesn't look like a valid Spotify Client ID (32 hex chars).\n\
             Also make sure your app lists http://127.0.0.1:{port}/login as a\n\
             Redirect URI (127.0.0.1, not localhost) or the link fails silently."
        ));
    }
    None
}

fn sync_and_wait(
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
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(*tracks));
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
    ToggleMono,
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
    SetSleepTimer(u32, bool),
    CancelSleepTimer,
    CheckHealth,
    /// One-shot async action, for the many picker keys whose only effect is a
    /// single fire-and-forget IPC call. Dispatched by [`Self::handle_command`]
    /// like any other command, so it is queued, ordered and drained off the
    /// render loop instead of spawning a detached task at every keypress.
    Fire(Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>),
}

impl TuiCommand {
    /// Queue a fire-and-forget async action without naming a variant for it.
    pub fn fire<Fut>(f: impl FnOnce() -> Fut + Send + 'static) -> Self
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::Fire(Box::new(move || Box::pin(f())))
    }
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

    /// The track on air for a live stream, as `(title, artist)`, or `None` for
    /// anything else so callers fall back to the ordinary track metadata.
    ///
    /// The daemon synthesises only the station name for a `radio://` track, so
    /// the real title comes from the stream's ICY `StreamTitle` and the artist
    /// from the station's tracklist — the only place the two are published
    /// separately.
    pub fn live_track(&self) -> Option<(String, String)> {
        let track = self.state.current_track.as_ref()?;
        if !is_live_stream(&track.path) {
            return None;
        }
        let title = self
            .state
            .radio_title
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(track.title.as_str());
        if title.is_empty() {
            return None;
        }
        let artist = self
            .state
            .radio_artist
            .as_deref()
            .unwrap_or(track.artist.as_str());
        let artist = if artist.is_empty() || artist == "Radio" {
            " ".to_string()
        } else {
            artist.to_string()
        };
        Some((title.to_string(), artist))
    }

    /// The read-only station tracklist the daemon last published for the
    /// playing `radio://` station, newest first. Empty for any other source,
    /// and for a station that publishes no tracklist — the queue then shows the
    /// station name as it always did.
    pub fn live_queue(&self) -> &[RadioTrack] {
        let live = self
            .state
            .current_track
            .as_ref()
            .is_some_and(|t| t.path.starts_with("radio://"));
        if !live {
            return &[];
        }
        &self.state.radio_tracks.tracks
    }

    fn next_lyrics_gen(&mut self) -> u64 {
        let g = self.lyrics.next_gen;
        self.lyrics.next_gen = self.lyrics.next_gen.wrapping_add(1).max(1);
        g
    }

    fn clear_search_previews(&mut self) {
        self.picker_preview_cover = None;
        self.picker_preview_stateful = None;
        self.picker_slot.clear();
        self.artist_cover = None;
        self.artist_cover_stateful = None;
        self.artist_slot.clear();
    }

    fn clear_preview(&mut self) {
        self.queue.preview_cover = None;
        self.queue.preview_cover_stateful = None;
        self.queue.preview_slot.clear();
    }

    fn clear_popup_cover(&mut self) {
        self.popup_track_id = None;
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        self.popup_slot.clear();
        self.spotify_popup_slot.clear();
    }

    pub async fn new(
        socket_path: &Path,
        setup_service: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = DaemonClient::connect(socket_path).await?;
        let state = DaemonState::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (pri_cmd_tx, pri_cmd_rx) = mpsc::unbounded_channel();
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
            tracks_cache_gen: 0,
            cached_albums: std::sync::Mutex::new(None),
            cached_artists: std::sync::Mutex::new(None),
            cached_genres: std::sync::Mutex::new(None),
            cached_folders: std::sync::Mutex::new(None),
            queue: QueueView {
                cache: Vec::new(),
                cursor: 0,
                move_index: None,
                move_target: 0,
                preview_cover: None,
                preview_cover_stateful: None,
                preview_slot: FetchSlot::default(),
                preview_fail_until: None,
            },
            browse_detail: None,
            yt_results_cache: Vec::new(),
            downloads: std::collections::HashMap::new(),
            downloading_urls: std::collections::HashSet::new(),
            playlist_cache: Vec::new(),
            most_played_cache: Vec::new(),
            recently_played_cache: Vec::new(),
            recently_added_cache: Vec::new(),
            playlist_tracks_cache: Vec::new(),
            spotify: SpotifyView {
                status: None,
                playlists: Vec::new(),
                playlist_tracks_cache: Vec::new(),
                search_results: Vec::new(),
                link_input: String::new(),
                sync_pending: false,
                synced_once: false,
                sync_announced: false,
                oauth_pending: false,
                oauth_url: None,
                oauth_error: None,
                oauth_port: "8990".to_string(),
                link_field: 0,
                search_debounce: None,
                search_loading: false,
                web_seq: 0,
                preview_cover: None,
                preview_cover_stateful: None,
                preview_fetch: FetchSlot::default(),
                preview_cache: std::collections::HashMap::new(),
                preview_shown: None,
            },
            charts: ChartsView::default(),
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
            footer_key_action: prefs.footer_key_action,
            audio_devices: Vec::new(),
            cover_provider: prefs.cover_provider.clone(),
            cover_cache_mb: prefs.cover_cache_mb,
            cover_cache_bytes: 0,
            about_viz: AboutViz::default(),
            auto_fetch_lyrics: prefs.auto_fetch_lyrics,
            icon_style: prefs.icon_style.clone(),
            crossfade_duration: 6,
            pending_delete: None,
            pending_prompt: None,
            yt_search_loading: false,
            yt_search_debounce: None,
            search_deadline: None,
            pickers: PickerManager::new(),
            sleep_timer: SleepTimerState {
                remaining: None,
                minutes: 30,
                input_mode: false,
                input_buf: String::new(),
                stop_immediately: true,
                focus: 0,
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
            pri_cmd_rx,
            pri_cmd_tx,
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
            path_display: None,
            prev_track_id: None,
            prev_status: PlaybackStatus::Stopped,
            prev_volume: 100,
            prev_cover_id: None,
            cover_art_dirty: false,
            data_dirty: false,
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
            extensions: prefs.extensions.clone(),
            selected_keys: std::collections::HashSet::new(),
            pending_motion: None,
            pending_track_ids: Vec::new(),
            pending_playlist_id: None,
            selected_track_ids: std::collections::HashSet::new(),
            playlist_creating: false,
            renaming_playlist: None,
            metadata: MetadataEditState {
                edit_track_ids: Vec::new(),
                fields: Default::default(),
                field_idx: 0,
                cover: None,
                cover_stateful: None,
                cover_dirty: false,
                cover_fetch: FetchSlot::default(),
            },
            pending_quit: false,
            mouse_map: MouseMap::default(),
            np_title_scroll: 0,
            track_anim_trigger: false,
            anim_fx: EffectManager::default(),
            track_popup_visible: false,
            popup_track_id: None,
            track_popup_cover: None,
            popup_cover_stateful: None,
            popup_slot: FetchSlot::default(),
            spotify_popup_slot: FetchSlot::default(),
            picker_preview_cover: None,
            picker_preview_stateful: None,
            picker_slot: FetchSlot::default(),
            artist_cover: None,
            artist_cover_stateful: None,
            artist_slot: FetchSlot::default(),
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
                offset_secs: 0.0,
            },
            zen: false,
            zen_surface: ZenSurface::Cover,
            show_health_panel: false,
            report_health: false,
            health_report: None,
            hide_help_bar: true,
            hide_footer: false,
            left_pane_lists: default_left_pane_lists(),
            show_preview: true,
            pending_suspend: false,
            setup: SetupView::default(),
            last_config_mtime: std::fs::metadata(prefs_path())
                .ok()
                .and_then(|m| m.modified().ok()),
        };
        if let Some(service) = setup_service.as_deref() {
            app.open_setup_picker(Some(service));
        }
        Ok(app)
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    pub fn send_high(&self, cmd: TuiCommand) {
        let _ = self.pri_cmd_tx.send(cmd);
    }

    /// Keys handled while Zen mode is active. Tab / Shift-Tab cycle the
    /// fullscreen surface (cover → visualizer → lyrics), `l` toggles the
    /// lyrics surface (fetching them first when needed), Space toggles
    /// playback, and z/Esc/q leave Zen mode. Every other key is swallowed
    /// so browsing/quit motions can't disturb the view.
    fn zen_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('z') => {
                self.zen = false;
                self.set_last_action("Leave Zen Mode", &key);
            }
            KeyCode::Tab => {
                self.zen_surface = self.zen_surface.next();
                self.set_last_action("Zen: Next Surface", &key);
            }
            KeyCode::BackTab => {
                self.zen_surface = self.zen_surface.prev();
                self.set_last_action("Zen: Prev Surface", &key);
            }
            KeyCode::Char('l') => {
                if self.zen_surface == ZenSurface::Lyrics {
                    self.zen_surface = ZenSurface::Cover;
                } else {
                    self.zen_surface = ZenSurface::Lyrics;
                    if self.lyrics.current.is_none() && !self.lyrics.fetching {
                        self.lyrics.fetching = true;
                        self.send_high(TuiCommand::FetchLyrics);
                    }
                }
            }
            KeyCode::Char(' ') => match self.state.status {
                PlaybackStatus::Playing => self.send_high(TuiCommand::Pause),
                PlaybackStatus::Paused => self.send_high(TuiCommand::PlayPause),
                PlaybackStatus::Stopped => {
                    if !self.queue.cache.is_empty() {
                        let idx = self.queue.cursor.min(self.queue.cache.len() - 1);
                        let path = self.queue.cache[idx].path.clone();
                        self.send_high(TuiCommand::Play(path));
                    }
                }
            },
            _ => {}
        }
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

    /// Record the footer's `KeyAction` echo for a command. `name` is the
    /// action's human label; `key` is what the user actually pressed, used
    /// instead when `footer_key_action` is `Keys`.
    fn set_last_action(&mut self, name: &str, key: &KeyEvent) {
        let label = match self.footer_key_action {
            FooterKeyAction::Action => name.to_string(),
            FooterKeyAction::Keys => format_key_event(key),
        };
        self.last_action_name = Some((
            label,
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    fn clamp_picker_selection(&mut self) {
        let (id, query) = match self.pickers.top_mut() {
            Some(t) => (t.id, t.query.clone()),
            None => return,
        };
        let max = match id {
            PickerId::Queue => {
                let live = self.live_queue().len();
                if live > 0 {
                    live.saturating_sub(1)
                } else {
                    self.queue.cache.len().saturating_sub(1)
                }
            }
            PickerId::YTSearch => self.yt_results_cache.len().saturating_sub(1),
            PickerId::SearchLibrary => self.search_library_picks().len().saturating_sub(1),
            PickerId::SpotifySearch => self.spot_picks().len().saturating_sub(1),
            PickerId::Equalizer => EQ_PRESETS.len().saturating_sub(1),
            PickerId::SleepTimer => 8,
            PickerId::Crossfade => 13,
            PickerId::VisualizerPreset => VisualizerPreset::all().len().saturating_sub(1),
            PickerId::FooterPreset => self.footer_presets.len().saturating_sub(1),
            PickerId::ProgressStyle => ProgressStyle::all().len().saturating_sub(1),
            PickerId::Notifications => self.notification_history.len().saturating_sub(1),
            PickerId::NotificationSettings => NotifType::ALL.len().saturating_sub(1),
            PickerId::PlaylistSelect => self.playlist_cache.len(),
            PickerId::PlaylistTrackSelect => self.tracks_cache.len().saturating_sub(1),
            PickerId::ThemePicker => self
                .themes
                .iter()
                .filter(|entry| fuzzy_match(&query, &entry.name))
                .count()
                .saturating_sub(1),
            PickerId::CommandPalette => CommandPalette::commands(&self.icon_style)
                .iter()
                .filter(|c| fuzzy_match(&query, c.icon))
                .count()
                .saturating_sub(1),
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
            PickerId::Queue => {
                let live = self.live_queue().len();
                if live > 0 { live } else { self.queue.cache.len() }
            }
            PickerId::YTSearch => self.yt_results_cache.len(),
            PickerId::SearchLibrary => self.search_library_picks().len(),
            PickerId::SpotifySearch => self.spot_picks().len(),
            PickerId::Equalizer => EQ_PRESETS.len(),
            PickerId::SleepTimer => 9,
            PickerId::Crossfade => 14,
            PickerId::VisualizerPreset => VisualizerPreset::all().len(),
            PickerId::FooterPreset => self.footer_presets.len(),
            PickerId::ProgressStyle => ProgressStyle::all().len(),
            PickerId::Notifications => self.notification_history.len(),
            PickerId::NotificationSettings => NotifType::ALL.len(),
            PickerId::PlaylistSelect => self.playlist_cache.len() + 1,
            PickerId::PlaylistTrackSelect => self.tracks_cache.len(),
            PickerId::ThemePicker => self
                .themes
                .iter()
                .filter(|entry| fuzzy_match(&query, &entry.name))
                .count(),
            PickerId::CommandPalette => CommandPalette::commands(&self.icon_style)
                .iter()
                .filter(|c| fuzzy_match(&query, c.icon))
                .count(),
            PickerId::PodcastFeeds => self.podcast.feeds.len(),
            PickerId::PodcastEpisodes => self.podcast.episodes.len(),
            PickerId::PodcastSubscribe => 1,
            PickerId::LoadStream => 1,
            PickerId::Radio => self.radio_picks().len(),
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
                    self.close_picker();
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

    /// Close the top picker, clearing per-picker state that Esc must reset
    /// (same cleanup for arrow-key closes, ).
    fn close_picker(&mut self) {
        if let Some(top) = self.pickers.top() {
            match top.id {
                PickerId::SleepTimer => self.sleep_timer.remaining = None,
                PickerId::SpotifyLink => {
                    // Closing the link picker ends any pending/cancelled flow.
                    self.spotify.oauth_pending = false;
                    self.spotify.oauth_url = None;
                    self.spotify.oauth_error = None;
                }
                PickerId::SpotifySearch => {
                    self.spotify.search_results.clear();
                    self.spotify.search_loading = false;
                }
                PickerId::EditMetadata => {
                    self.metadata.cover = None;
                    self.metadata.cover_stateful = None;
                    self.metadata.edit_track_ids.clear();
                    self.metadata.cover_fetch.clear();
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
                    self.selected_track_ids.clear();
                }
                PickerId::PlaylistSelect => {
                    self.playlist_creating = false;
                    self.renaming_playlist = None;
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
            self.close_picker();
            return;
        };
        let track_ids: Vec<i64> = self.selected_track_ids.iter().copied().collect();
        if track_ids.is_empty() {
            self.notify_typed(
                "System",
                "No tracks selected — playlist stays empty",
                NotificationKind::Info,
                false,
                NotifType::NowPlaying,
            );
            self.close_picker();
            self.pending_playlist_id = None;
            self.selected_track_ids.clear();
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
        self.close_picker();
        self.pending_playlist_id = None;
        self.selected_track_ids.clear();
    }

    async fn handle_paste(&mut self, text: &str) {
        if let Some(top) = self.pickers.top_mut() {
            match top.id {
                PickerId::SpotifySearch => {
                    if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                        // Unlinked: no manual token input anymore.
                    } else {
                        top.query.push_str(text);
                        self.spotify.search_results.clear();
                        self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                        self.spotify.search_debounce = Some(
                            std::time::Instant::now() + Duration::from_millis(SEARCH_DEBOUNCE_MS),
                        );
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
}

pub mod charts;
pub mod cmd;
pub mod cover;
pub mod keys;
pub mod lastfm;
pub mod lyrics;
pub mod notify;
pub mod podcast;
pub mod prefs;
pub mod radio;
pub mod run;
pub mod search;
pub mod spotify;
pub mod state;
pub mod theme;

#[cfg(test)]
mod tests;

// One glob per submodule: a leaf needs a single `use crate::app::*;`
// instead of importing each shared item itself.
pub(crate) use charts::*;
pub(crate) use cover::*;
pub(crate) use keys::*;
pub(crate) use lastfm::*;
pub(crate) use lyrics::*;
pub(crate) use notify::*;
pub(crate) use podcast::*;
pub(crate) use prefs::*;
pub(crate) use radio::*;
pub(crate) use run::*;
pub(crate) use search::*;
pub(crate) use spotify::*;
pub(crate) use state::*;
pub(crate) use theme::*;
