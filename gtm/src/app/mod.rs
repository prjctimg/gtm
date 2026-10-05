// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Application state machine: input handling, IPC dispatch, crossfade
//
// This is free software released under the GPL-3.0 license.
// The imports below are this module's single shared import set; each
// submodule reaches them with one `use crate::app::*;`, so a few are
// unused here by design.
#![allow(unused_imports)]
pub(crate) use std::future::Future;
pub(crate) use std::path::Path;
pub(crate) use std::pin::Pin;
pub(crate) use std::time::Duration;

pub(crate) use crate::providers::spotify::SpotifyRemote;
pub(crate) use crate::shared::client::{DaemonClient, LastfmStatus};
pub(crate) use crate::shared::custom::CustomRadioStation;
pub(crate) use crate::shared::global::{DaemonState, EqPreset, PlaybackStatus, RepeatMode};
pub(crate) use crate::shared::ipc::{CacheKind, DaemonEvent, DaemonRes, HealthReport, SyncKind};
pub(crate) use crate::shared::log::log;
pub(crate) use crate::shared::podcast::{
    PodcastEpisode, PodcastFeed, PodcastResult, PodcastStatus,
};
pub(crate) use crate::shared::radio::{RadioCountry, RadioStation, RadioTag, RadioTrack};
pub(crate) use crate::shared::secret::{SPOTIFY_CLIENT_ID, get_secret, set_secret};
pub(crate) use crate::shared::spotify::{
    LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifySearchKind, SpotifyStatus, SpotifyTrack,
};
pub(crate) use crate::shared::state::{ThemeMode, TrackSort, path_is_remote};
pub(crate) use crate::shared::track::{LrcData, LrcLine, Playlist, TrackInfo, YTSearchResult};
pub(crate) use crate::shared::{CoreError, MAX_VOLUME, MetadataPatch};
pub(crate) use crossterm::event::{
    self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
pub(crate) use ratatui::Terminal;
pub(crate) use ratatui::layout::Alignment;
pub(crate) use ratatui::widgets::Paragraph;
pub(crate) use ratatui_image::picker::Picker;
pub(crate) use ratatui_image::protocol::StatefulProtocol;
pub(crate) use tachyonfx::EffectManager;
pub(crate) use tokio::sync::mpsc;

pub(crate) use base64::Engine;

pub(crate) use crate::extensions::{ExtensionId, ExtensionsConfig};
pub(crate) use crate::footer::{
    FooterCache, FooterKeyAction, FooterPreset, is_live_stream, merged_presets,
};
pub(crate) use crate::keymap::{
    BoundCommand, KeyContext, Keybindings, KeyboardAction, default_clash_warnings,
    default_keybindings, detect_clashes, format_key_event, parse_key_event,
};
pub(crate) use crate::mouse::{MouseMap, MouseZone};
pub(crate) use crate::oauth::open_browser;
pub(crate) use crate::picker::{PickerId, PickerManager, PickerSource};
pub(crate) use crate::progress::{ProgressSmoother, ProgressStyle};
pub(crate) use crate::reactive::{ReactivePalette, derive_theme, extract_palette};
pub(crate) use crate::theme::{
    AppTheme, ThemeEntry, blend_colors, chadrula, detect_os_theme, merged_themes,
};
pub(crate) use crate::ui;
pub(crate) use crate::ui::{
    CROSSFADE_DURATIONS, Command, CommandPalette, HELP_LINES, cover_provider_label,
    theme_mode_label, use_nerd_fonts,
};
pub(crate) use crate::visualizer::{AudioVisualizer, VisualizerPreset};
pub const NUM_SETTINGS_CATEGORIES: usize = 3;
/// The library views, in the order the left pane lists them.
///
/// All Tracks, Albums, Artists and Genres were four of these. They are one view
/// now with a filter over it: they were four renderings of the same question —
/// what is in the library, grouped one way or another — and the left pane spent
/// four rows on the answer while the pane below showed one of them. The filter
/// is [`LibraryFilter`], switched with `[` and `]`.
///
/// Named rather than compared as bare numbers: these indices are the row's
/// identity in a dozen places, and a renumbering is otherwise invisible.
pub const LIB_CATEGORIES: &[&str] = &[
    "Library",
    "Playlists",
    "Spotify",
    "Radio",
    "Top Charts",
    "Podcasts",
];
/// The old name, for the places that only pass the list on.
pub const LIBRARY_CATEGORIES: &[&str] = LIB_CATEGORIES;
pub const LIB_ALL: usize = 0;
pub const LIB_PLAYLISTS: usize = 1;
pub const LIB_SPOTIFY: usize = 2;
pub const LIB_RADIO: usize = 3;
pub const LIB_CHARTS: usize = 4;
pub const LIB_PODCASTS: usize = 5;

/// Every name that is no longer a left-pane row of its own, in the view that
/// took it.
///
/// `Liked` was the last: it is a list of tracks by nothing more than a flag on
/// them, which is what a playlist is too, so it is a group of Playlists rather
/// than a row of its own. A `left_pane_lists` saved before any of these moves
/// names the list by string, so `clean_left_pane` rewrites it to the row that
/// now holds the content instead of dropping the list it named.
pub const LIB_ABSORBED: &[&str] = &[
    "All Tracks",
    "Albums",
    "Artists",
    "Genres",
    "Folders",
    "Most Played",
    "Recently Played",
    "Recently Added",
    "Liked",
];

/// The three library lists the Playlists view groups, named rather than
/// numbered.
///
/// These were left-pane categories once, and the fetch that fills them was
/// dispatched on those category indices. That is how `Most Played` — category
/// 5 — came to be requested as 7 and came back as a different list: the
/// indices outlived the rows, and by the time the rows were gone so were the
/// indices, leaving two live categories sharing a number with a dead one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistList {
    Most,
    Recent,
    Added,
}

/// How the Library view groups the same rows.
///
/// `[` and `]` walk it. Every arm reads the same data, so the filter is the
/// only difference between them: which column the rows are keyed by.
/// How the Playlists view groups its rows.
///
/// The three history lists were top-level categories next to Playlists, which
/// put a *slice* of the library in the same left-pane list as the views you
/// browse it with — four rows to flip between to reach a playlist, and four
/// more to reach what you last played. They are lists the same way a playlist
/// is: named collections of tracks, one row each, Enter to open. So they live
/// here, as groups of the one view that already had that shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistGroup {
    /// Playlists the user made. Opening one drills into its tracks.
    Playlists,
    /// Ranked by play count. Opening one lists the tracks.
    MostPlayed,
    /// Most recent first, by last-played time.
    RecentlyPlayed,
    /// Most recent first, by library insertion order.
    RecentlyAdded,
    /// Favourited tracks, local and provider alike.
    Liked,
}

impl PlaylistGroup {
    pub const ALL: [PlaylistGroup; 5] = [
        PlaylistGroup::Playlists,
        PlaylistGroup::MostPlayed,
        PlaylistGroup::RecentlyPlayed,
        PlaylistGroup::RecentlyAdded,
        PlaylistGroup::Liked,
    ];

    /// Name shown on the header, so the pane says which group is on screen.
    pub fn label(self) -> &'static str {
        match self {
            PlaylistGroup::Playlists => "Playlists",
            PlaylistGroup::MostPlayed => "Most Played",
            PlaylistGroup::RecentlyPlayed => "Recently Played",
            PlaylistGroup::RecentlyAdded => "Recently Added",
            PlaylistGroup::Liked => "Liked",
        }
    }

    /// The library list whose fetch populates this group, for the three history
    /// rows. `Playlists` has none — the daemon pushes that list — and `Liked`
    /// is one fetch of its own, so it answers separately.
    pub fn hist(self) -> Option<HistList> {
        match self {
            PlaylistGroup::MostPlayed => Some(HistList::Most),
            PlaylistGroup::RecentlyPlayed => Some(HistList::Recent),
            PlaylistGroup::RecentlyAdded => Some(HistList::Added),
            PlaylistGroup::Playlists | PlaylistGroup::Liked => None,
        }
    }

    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|g| *g == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let at = Self::ALL.iter().position(|g| *g == self).unwrap_or(0);
        Self::ALL[(at + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// What an empty group says, since the four do not fill up the same way: a
    /// playlist is empty because the user has not made one, a history list is
    /// empty because nothing has been played or scanned.
    pub fn empty_hint(self) -> &'static str {
        match self {
            PlaylistGroup::Playlists => "Hint: press A to add the highlighted track to a playlist",
            PlaylistGroup::MostPlayed => "Hint: play counts build up as you listen",
            PlaylistGroup::RecentlyPlayed => "Hint: play any track and it will show up here",
            PlaylistGroup::RecentlyAdded => "Hint: add music to your library to see it here",
            PlaylistGroup::Liked => "Hint: press f on a track to keep it here",
        }
    }
}

/// The row label a history group shows for a track: title, with the artist
/// disambiguating the repeats a ranked list is full of.
///
/// A bare title makes "Intro" indistinguishable from every other "Intro", which
/// is the one thing a list ordered by play count or last-played time cannot
/// afford — the order is the whole point of those rows.
pub(crate) fn playlist_row_label(t: &TrackInfo) -> String {
    let title = if t.title.is_empty() {
        t.display_title()
    } else {
        t.title.clone()
    };
    if t.artist.is_empty() {
        title
    } else {
        format!("{title} — {}", t.artist)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryFilter {
    Tracks,
    Albums,
    Artists,
    Genres,
    /// One row per directory holding at least one track.
    ///
    /// A grouping rather than a category of its own: it keys off `path`, so it
    /// is the same "one row, many tracks behind it" shape the other three are,
    /// and it answers the same drill-down. Folders used to be a top-level
    /// category, which put a *slice* of the library in the same list as the
    /// views you browse it with.
    Folders,
}

impl LibraryFilter {
    /// The filters in `[` / `]` order.
    pub const ALL: [LibraryFilter; 5] = [
        LibraryFilter::Tracks,
        LibraryFilter::Albums,
        LibraryFilter::Artists,
        LibraryFilter::Genres,
        LibraryFilter::Folders,
    ];

    /// The same name, singular and plural, for counts and prose ("12 albums",
    /// "delete every track in this album?"). `&'static str` because the counts
    /// are built at draw time and must not allocate to name a row count.
    pub fn one(self) -> &'static str {
        match self {
            LibraryFilter::Tracks => "track",
            LibraryFilter::Albums => "album",
            LibraryFilter::Artists => "artist",
            LibraryFilter::Genres => "genre",
            LibraryFilter::Folders => "folder",
        }
    }

    pub fn many(self) -> &'static str {
        match self {
            LibraryFilter::Tracks => "tracks",
            LibraryFilter::Albums => "albums",
            LibraryFilter::Artists => "artists",
            LibraryFilter::Genres => "genres",
            LibraryFilter::Folders => "folders",
        }
    }

    /// Name shown on the header, so the pane says which grouping is on screen.
    pub fn label(self) -> &'static str {
        match self {
            LibraryFilter::Tracks => "Tracks",
            LibraryFilter::Albums => "Albums",
            LibraryFilter::Artists => "Artists",
            LibraryFilter::Genres => "Genres",
            LibraryFilter::Folders => "Folders",
        }
    }

    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let at = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(at + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}
/// Sanitize a TOML `left_pane_lists` value: keep only canonical category
/// names, drop duplicates, preserve user order. Empty (or fully unknown)
/// input falls back to the full default set so the pane always renders.
pub fn clean_left_pane(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        // The four list names the Library view absorbed all mean the same row
        // now, so a config saved before the merge keeps one of them rather
        // than silently losing the list it named.
        let name = match n.as_str() {
            "All Tracks" | "Albums" | "Artists" | "Genres" => LIB_CATEGORIES[LIB_ALL],
            other => other,
        };
        // Folders is the same story one view further along: a config that named
        // it keeps the Library row, which is where its content now lives. Liked
        // is a group of Playlists, so it keeps that row.
        let name = match name {
            "Folders" => LIB_CATEGORIES[LIB_ALL],
            "Liked" => LIB_CATEGORIES[LIB_PLAYLISTS],
            other => other,
        };
        if LIB_CATEGORIES.contains(&name) && !out.iter().any(|e| e == name) {
            out.push(name.to_string());
        }
    }
    if out.is_empty() {
        left_pane_defaults()
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
    pub(crate) last_display_position: f64,
    /// Raw (guarded, un-smoothed) daemon playback position used for
    /// time-synced lyric matching so the active verse updates without the
    /// EMA lag that smooths the progress bar.
    pub(crate) raw_position: f64,
    /// Set when a seek is issued so the monotonic position guard is skipped
    /// (a backward seek would otherwise be clamped and never re-sync the lyric
    /// highlight). Cleared shortly after the seek lands.
    pub(crate) seek_pending: Option<std::time::Instant>,
    /// Coalesced seek commands: while the user holds a seek key (long-press),
    /// each repeat press adjusts `seek_cmd_accum` and the local position only.
    /// The daemon receives a single, debounced seek (via `ensure_seek_flush`)
    /// once the repeats settle — so it never does a full re-decode per keypress,
    /// which is what surfaced errors on long-press seeking.
    pub(crate) seek_cmd_accum: Option<f64>,
    pub(crate) last_seek_press: Option<std::time::Instant>,
    pub progress_smoother: ProgressSmoother,
    pub(crate) last_frame: std::time::Instant,
    pub frame_count: u64,
    /// Progress whip scanner position (Knight Rider style).
    pub scanner_pos: i32,
    /// Scanner direction: 1 = forward, -1 = backward.
    pub scanner_dir: i32,
    /// Hold frames at each end before reversing.
    pub scanner_hold: i32,
    /// Cursor blink toggle for search pickers (alternates every ~8 frames).
    pub cursor_blink: bool,
    /// Set by a terminal resize, cleared by the frame that draws at the new
    /// size. The next draw has to start from an empty buffer.
    pub(crate) resized: bool,
    /// When the user last pressed a key, pasted, or clicked. The idle clock
    /// behind daydreaming; daemon traffic deliberately does not touch it.
    pub(crate) last_input: std::time::Instant,
    /// True while the TUI has been idle long enough to daydream, i.e. to show
    /// the visualizer over the library view.
    pub daydreaming: bool,
    /// Seconds of inactivity before daydreaming starts. A preview is the wrong
    /// answer to "what is playing": it covers the library and the now-playing
    /// pane, both of which name the track, and it has no controls. A minute is
    /// long enough to be an absence rather than a pause.
    pub(crate) daydream_secs: u64,
    pub input_mode: InputMode,
    pub search_query: String,
    /// Per-view selection index: one slot per library view, plus one per
    /// Library filter. The filter is a mode of one view, so it needs its own
    /// slot the way each view has its own — switching to Albums and back has
    /// to land on the row that view was left on, not on whichever row the
    /// filter before it happened to point at.
    pub(crate) scroll_offset: [usize; LIB_CATEGORIES.len() + LibraryFilter::ALL.len()],
    pub library_category: usize,
    /// Grouping of the Library view. Only meaningful on [`LIB_ALL`].
    pub library_filter: LibraryFilter,
    /// Grouping of the Playlists view. Only meaningful on [`LIB_PLAYLISTS`],
    /// and the only group that drills into named playlists; the three history
    /// groups list their tracks directly, so `browse_detail` stays `None` there.
    pub playlist_group: PlaylistGroup,
    /// Whether the active group's rows are on screen, or the list of groups is.
    ///
    /// Two levels, because one level is not a list: the five groups were five
    /// renderings of the same pane switched with `[` / `]`, so the list of them
    /// was never on screen and the left-pane row went on naming Playlists
    /// whatever was showing. `browse_detail` cannot carry the outer level
    /// either, because it also names the playlist whose tracks are showing —
    /// one field, three levels, and the outer one would have to be spelled as a
    /// playlist name.
    pub playlist_open: bool,
    pub library_pane_focus: bool,
    pub settings_category: usize,
    pub settings_pane_focus: bool,
    pub settings_option: usize,
    pub tracks_cache: Vec<TrackInfo>,
    /// Generation bumped on every wholesale `tracks_cache` replacement; keys
    /// the `unique_*` caches below so per-frame renders don't rebuild maps.
    pub(crate) tracks_cache_gen: u64,
    pub(crate) cached_albums: CachedCategories,
    pub(crate) cached_artists: CachedCategories,
    pub(crate) cached_genres: CachedCategories,
    pub(crate) cached_folders: CachedCategories,
    pub queue: QueueView,
    /// Identity of the row the library is drilled into.
    ///
    /// For every local category this *is* the display name. Spotify is the
    /// exception: it stores the bare playlist id, because that is what
    /// `play_all` and `resolve` look the playlist up by, and the header used to
    /// render it directly — so drilling into a playlist showed
    /// `▶ 37i9dQZF1DXcBWIGoYBM5M`, and `liked-songs` showed literally.
    pub browse_detail: Option<String>,
    /// Display name for [`Self::browse_detail`], where the key is not a name.
    /// `None` means the detail is its own title.
    pub browse_title: Option<String>,
    pub yt_results_cache: Vec<YTSearchResult>,
    pub playlist_cache: Vec<Playlist>,
    pub most_played_cache: Vec<TrackInfo>,
    /// Favourites as the daemon holds them, local files and provider rows alike.
    pub fav_cache: Vec<TrackInfo>,
    pub recently_played_cache: Vec<TrackInfo>,
    pub recently_added_cache: Vec<TrackInfo>,
    pub playlist_tracks_cache: Vec<TrackInfo>,
    /// Every track in every Spotify playlist, deduplicated — what the
    /// "All Tracks" list shows. Rebuilt when a playlist sync lands; see
    /// `App::playlist_union`.
    pub playlist_tracks: Vec<TrackInfo>,
    pub spotify: SpotifyView,
    pub charts: ChartsView,
    /// The cover-grid mode of the album, artist and genre lists.
    pub grid: GridView,
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
    /// Discord application id for Rich Presence; the Setup chooser edits it.
    pub discord_id: Option<String>,
    /// Last reported cover cache disk usage in bytes.
    pub cover_cache_bytes: u64,
    /// About window: decorative visualization state (preset rotates, waveform
    /// is synthetic so it animates even when nothing is playing).
    pub about_viz: AboutViz,
    /// Whether to automatically fetch lyrics on track change
    pub auto_fetch_lyrics: bool,
    /// Icon style for command palette: "mdi" (Material Design Icons) or "emoji"
    pub icon_style: String,
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
    /// The live `StreamTitle` seen on the previous frame, used to spot the
    /// track on air advancing without a path change.
    pub live_title: Option<String>,
    pub terminal_cols: u16,
    pub terminal_rows: u16,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    pub(crate) cmd_tx: mpsc::Sender<TuiCommand>,
    pub(crate) pri_cmd_rx: mpsc::UnboundedReceiver<TuiCommand>,
    pub(crate) pri_cmd_tx: mpsc::UnboundedSender<TuiCommand>,
    pub(crate) ipc_rx: mpsc::UnboundedReceiver<IpcResult>,
    pub(crate) ipc_tx: mpsc::UnboundedSender<IpcResult>,
    pub(crate) keybindings: Keybindings,
    pub(crate) prefs_keybindings: std::collections::HashMap<String, String>,
    pub theme_index: usize,
    pub list_scroll: usize,
    /// Scroll offset for the left pane's category list.
    ///
    /// A second offset because the left pane scrolls independently of the
    /// results pane: it holds every category, the results pane holds one
    /// category's rows, and they are not the same length in either direction.
    pub left_list_scroll: usize,
    pub viewport_items: usize,
    pub transparent_bg: bool,
    pub transparent_pickers: bool,
    pub reactive_theme: bool,
    pub reactive_theme_intensity: f32,
    pub(crate) reactive_palette: Option<ReactivePalette>,
    /// Cover generation the in-flight palette extraction belongs to, so a
    /// reply for a cover already replaced cannot tint the UI.
    pub(crate) reactive_gen: Option<u64>,
    pub last_action_name: Option<(String, std::time::Instant)>,
    pub footer_title_scroll: usize,
    /// strftime-style format string for the footer `Time` module.
    pub footer_time_format: String,
    /// OS-theme compliance mode: "auto" (detect dark/light), "dark", "light".
    pub theme_mode: String,
    /// How the library track list is sorted.
    pub track_sort: TrackSort,
    pub is_ready: bool,
    pub(crate) last_queue_cursor: u64,
    /// Set when the user manually triggers Next/Prev so the "Up next"
    /// notification only appears on genuine auto-advance.
    pub manual_track_advance: bool,
    /// True for the current track change if it was automatic (not a manual
    /// Next/Prev/seek). Captured when PlaybackStarted is drained, before the
    /// manual-advance flag is reset, so the dust animation can be gated to
    /// genuine auto-advances only.
    pub auto_track_advance: bool,
    pub(crate) path_display: Option<String>,
    /// Whether the idle library reset has already run for the current spell of
    /// "queue empty and nothing playing". Latched so it fires on the
    /// transition, not on every poll that observes the same idle state.
    pub(crate) idle_reset: bool,
    /// Whether the IPC link is currently up, so a reconnect is handled on its
    /// rising edge rather than continuously.
    pub(crate) link_up: bool,
    /// Last value each of these reconciliations saw. A difference is what
    /// triggers a full sweep of the frame's dirty flags -- see
    /// [`App::mark_all_dirty`].
    pub(crate) prev_track_id: Option<i64>,
    pub(crate) prev_status: PlaybackStatus,
    pub(crate) prev_volume: u8,
    pub(crate) prev_cover_id: Option<i64>,
    /// Set when something has invalidated the reconciliation itself: a
    /// reconnect, a state resnapshot, a resize. The next frame refreshes every
    /// widget rather than trusting that the per-field trackers still agree with
    /// the state, which is the assumption a tracker cannot check.
    ///
    /// Latched rather than acted on in place, because the place that needs to
    /// know is the frame loop and the thing that knows is an event handler.
    pub full_sync: bool,
    pub(crate) cover_art_dirty: bool,
    /// Set whenever the daemon pushes events or refreshes state so the next
    /// frame re-renders even if no visual trigger (position/animation) is
    /// active yet. Cleared after each forced render.
    pub(crate) data_dirty: bool,
    pub footer_cache: FooterCache,
    pub footer_presets: Vec<FooterPreset>,
    pub footer_preset: usize,
    pub(crate) last_event_time: std::time::Instant,
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
    pub(crate) pending_motion: Option<char>,
    pub pending_track_ids: Vec<i64>,
    /// Id of a freshly-created playlist awaiting track selection.
    pub pending_playlist_id: Option<i64>,
    /// Tracks currently highlighted for the in-flight new-playlist flow.
    pub selected_track_ids: std::collections::HashSet<i64>,
    /// The `"{artist} - {title}"` of the live track the Spotify destination
    /// picker is filing, held between opening the picker and the write.
    pub live_query: Option<String>,
    /// Destinations ticked in that picker. The empty string is the Liked Songs
    /// row, which has no playlist id.
    pub live_dests: std::collections::HashSet<String>,
    /// Resolved Spotify track URI for `live_query`, so repeated picks do not
    /// re-search.
    pub live_uri: Option<String>,
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
    pub(crate) popup_slot: FetchSlot<i64>,
    /// In-flight fetch slot for the Spotify drill-down popup cover, keyed by
    /// the track's album-image URL instead of a local library id.
    pub(crate) spotify_popup_slot: FetchSlot<String>,
    /// Cover art for the SearchLibrary picker preview window.
    pub picker_preview_cover: Option<Vec<u8>>,
    pub picker_preview_stateful: Option<StatefulProtocol>,
    pub(crate) picker_slot: FetchSlot<i64>,
    /// Cover art for artist selections in the search picker preview.
    pub artist_cover: Option<Vec<u8>>,
    pub artist_cover_stateful: Option<StatefulProtocol>,
    pub(crate) artist_slot: FetchSlot<String>,
    // Monotonic generation counter for all cover fetches — disambiguates
    // stale responses and `id == 0` reuse across different tracks.
    pub(crate) next_cover_gen: u64,
    pub lyrics: LyricsView,
    /// Zen-mode flag: fullscreen now-playing / visualizer surfaces.
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
    pub(crate) last_config_mtime: Option<std::time::SystemTime>,
}

pub(crate) enum IpcResult {
    RefreshDone(Box<DaemonState>, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>, u64),
    PopupCoverArt(Option<Vec<u8>>, i64, u64),
    /// Cover bytes for one cell of the album/artist/genre grid, as
    /// (bytes, representative track id, grid round).
    ///
    /// Separate from `PopupCoverArt` rather than sharing it: the grid keeps
    /// dozens of covers on screen at once and none of them is "the" preview, so
    /// a single-slot reply would have every cell overwrite the last one's slot
    /// and the pane would show one cover in forty places.
    GridCover(Option<Vec<u8>>, i64, u64),
    QueuePreviewCover(Option<Vec<u8>>, String, u64),
    /// Text read from the system clipboard for a form field, named because the
    /// form may have closed while the paste tool was running.
    ClipboardPaste(ClipField, String),
    PickerPreviewCover(Option<Vec<u8>>, i64, u64),
    MetadataCoverArt(Option<Vec<u8>>, i64, u64),
    ArtistCoverArt(Option<Vec<u8>>, String, u64),
    SpotifyPreviewCover(Option<Vec<u8>>, String, u64),
    /// Cover art for the highlighted playlist drill-down row, as
    /// (bytes, image url, fetch generation).
    SpotifyRowCover(Option<Vec<u8>>, String, u64),
    /// Cover art for the highlighted playlist, as (bytes, image url, generation).
    SpotifyListCover(Option<Vec<u8>>, String, u64),
    /// Album cover bytes for the highlighted Spotify drill-down row, keyed by
    /// its image URL (guarded via `spotify_popup_slot`).
    SpotifyPopupCover(Option<Vec<u8>>, String, u64),
    CoverPicker(Option<Picker>),
    /// A lyrics reply: the lines, the generation that asked for them, and the
    /// path of the track it was asked for.
    ///
    /// The generation alone is not enough. It orders a reply against a *later
    /// fetch*, so it catches two requests racing each other — but not the case
    /// that actually shows up: the track changes, no new fetch has started yet
    /// (the change arrives through the same drain that would have delivered the
    /// reply), and the reply for the previous track still matches the live
    /// generation and gets written. The path says which track the lines belong
    /// to, which no generation can.
    Lyrics(Option<LrcData>, u64, Option<String>),
    /// A podcast episode's transcript, shown in the lyrics pane.
    ///
    /// Deliberately not `Lyrics`: that variant is gated on a per-track
    /// generation so a response that arrives after the track changed is
    /// dropped. A transcript is not for the track that happens to be playing —
    /// it was asked for by name — so gating it would throw away exactly the
    /// fetch the user waited for.
    PodcastTranscript(Option<LrcData>, String),
    LibraryTracks(Vec<TrackInfo>),
    MostPlayed(Vec<TrackInfo>),
    Favourites(Vec<TrackInfo>),
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
    /// A free-text track query resolved to a Spotify URI, for the live
    /// track's like and add-to-playlist actions.
    SpotifyMatch(String),
    ReactivePalette(Option<ReactivePalette>, u64),
    PodcastStatus(Option<PodcastStatus>),
    PodcastFeeds(Vec<PodcastFeed>),
    PodcastSearch(Vec<PodcastResult>),
    PodcastEpisodes(Vec<PodcastEpisode>),
    RadioSearch(Vec<RadioStation>),
    RadioTop(Vec<RadioStation>),
    RadioTags(Vec<RadioTag>),
    RadioCountries(Vec<RadioCountry>),
    RadioBrowseStations(Vec<RadioStation>),
    /// A station tracklist pulled on demand when the queue view opens before
    /// the daemon's refresh tick has landed.
    RadioTracklist(crate::shared::radio::RadioTracklist),
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
pub(crate) fn self_err(ipc_tx: &mpsc::UnboundedSender<IpcResult>, msg: String) {
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

/// Read the system clipboard, using the same CLI ladder as
/// [`copy_to_clipboard`] and for the same reason: it is one platform tool per
/// desktop, and a crate would be a dependency for two shell-outs.
///
/// Blocking where the copy is not: these tools exit once they have served the
/// request rather than detaching, so the caller runs this off the UI thread.
/// X11 needs `xclip -selection clipboard` — its default selection is PRIMARY,
/// which is the middle-click buffer, not the one every paste reads.
pub(crate) async fn paste_from_clipboard() -> Result<String, String> {
    let (tool, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("pbpaste", &[])
    } else if cfg!(target_os = "windows") {
        ("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        ("wl-paste", &["--no-newline"])
    } else if std::env::var_os("DISPLAY").is_some() {
        ("xclip", &["-selection", "clipboard", "-o"])
    } else {
        ("wl-paste", &["--no-newline"])
    };
    let tool = tool.to_string();
    let name = tool.clone();
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let out =
        tokio::task::spawn_blocking(move || std::process::Command::new(tool).args(&args).output())
            .await
            .map_err(|e| format!("clipboard task failed: {e}"))?
            .map_err(|e| format!("`{name}` unavailable: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`{}` failed: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Open the OAuth URL in a browser. When no opener works the authorize URL is
/// already rendered inline in the link picker, so only a quiet fallback notice
/// is recorded (no floating card). Non-blocking.
pub(crate) fn try_open_browser(url: &str, ipc_tx: &mpsc::UnboundedSender<IpcResult>) {
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

    /// Background for a floating notification.
    ///
    /// Never transparent, unlike every other floating surface. A notification
    /// is the one thing that appears over whatever the user happens to be
    /// looking at — often for less than two seconds, often with an error in it,
    /// and always while the thing that caused it is still on screen. Blending it
    /// 50% into the background put the library or the cover art straight through
    /// the message, and the part of the text that lost the contrast was the part
    /// that mattered. The card is opaque; the *theme* it is opaque in is still
    /// reactive, because the theme is what supplies the colour.
    pub fn notification_bg(&self) -> ratatui::style::Color {
        self.theme.elevated_bg
    }

    pub fn chrome_bg(&self) -> ratatui::style::Color {
        if self.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            self.theme.border
        }
    }

    /// Background for the Zen surface: the reactive palette, washed in harder than
    /// the rest of the app.
    ///
    /// The app's own surface is already washed with the palette at
    /// `reactive_theme_intensity`, so on a fullscreen surface with no panes to
    /// tell it apart from, Zen came up looking like the library it covers. The
    /// extra wash is what separates the two.
    ///
    /// There used to be a lift toward white on top of it, and the wash was more
    /// than twice this deep. Together they made the surface a pale tint of the
    /// artwork — on a light cover the whole view was near-white with a theme
    /// foreground drawn on it, which is where the lyric contrast complaint came
    /// from. Nothing is lifted now: the artwork sits on the wash.
    pub fn zen_bg(&self) -> ratatui::style::Color {
        // Both constants were higher: a 0.55 wash followed by a lift toward
        // white left the Zen surface a pale tint of the artwork, so on a light
        // cover the whole view was a wash of near-white with a theme foreground
        // drawn on it, and the lyric lines lost the contrast they have in the
        // pane. A quarter of the palette is enough to tell the Zen surface from
        // the app without lighting it up, and the lift is gone — the artwork
        // sits on the wash rather than on top of a brightened version of it.
        const LIFT: f64 = 0.0;
        const WASH: f64 = 0.25;
        let base = self.surface_bg();
        let washed = match self.reactive_palette.filter(|_| self.reactive_theme) {
            Some(pal) => blend_colors(
                base,
                ratatui::style::Color::Rgb(pal.primary[0], pal.primary[1], pal.primary[2]),
                WASH,
            ),
            None => base,
        };
        blend_colors(washed, ratatui::style::Color::Rgb(255, 255, 255), LIFT)
    }

    pub(crate) fn next_cover_gen(&mut self) -> u64 {
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

    /// Whether the live track on air changed since the last frame. The queue
    /// path is stable across a whole station session, so this is the only
    /// signal that a new track — and therefore new artwork — has started.
    fn live_advanced(&mut self) -> bool {
        let now = self.state.radio_title.clone();
        if now != self.live_title {
            self.live_title = now;
            return true;
        }
        false
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

    pub(crate) fn clear_popup_cover(&mut self) {
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
            resized: false,
            last_input: std::time::Instant::now(),
            daydreaming: false,
            daydream_secs: DEFAULT_DAYDREAM_SECS,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            scroll_offset: [0; LIB_CATEGORIES.len() + LibraryFilter::ALL.len()],
            library_category: LIB_ALL,
            library_filter: LibraryFilter::Tracks,
            playlist_group: PlaylistGroup::Playlists,
            playlist_open: false,
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
            browse_title: None,
            yt_results_cache: Vec::new(),
            downloads: std::collections::HashMap::new(),
            downloading_urls: std::collections::HashSet::new(),
            playlist_cache: Vec::new(),
            most_played_cache: Vec::new(),
            fav_cache: Vec::new(),
            recently_played_cache: Vec::new(),
            recently_added_cache: Vec::new(),
            playlist_tracks_cache: Vec::new(),
            playlist_tracks: Vec::new(),
            spotify: SpotifyView {
                status: None,
                playlists: Vec::new(),
                playlist_tracks_cache: Vec::new(),
                search_results: Vec::new(),
                sync_pending: false,
                synced_once: false,
                sync_announced: false,
                oauth_pending: false,
                oauth_url: None,
                oauth_error: None,
                oauth_port: "8990".to_string(),
                oauth_client_id: String::new(),
                oauth_field: 0,
                oauth_form_error: None,
                oauth_sent_id: None,
                search_debounce: None,
                search_loading: false,
                web_seq: 0,
                preview_cover: None,
                preview_cover_stateful: None,
                preview_fetch: FetchSlot::default(),
                preview_cache: std::collections::HashMap::new(),
                preview_shown: None,
                preview_fail_until: std::collections::HashMap::new(),
                row_cover: None,
                row_cover_stateful: None,
                row_fetch: FetchSlot::default(),
                row_cover_index: None,
                row_shown: None,
                prefetched_for: None,
                list_cover: None,
                list_cover_stateful: None,
                list_fetch: FetchSlot::default(),
                list_shown: None,
            },
            charts: ChartsView::default(),
            grid: GridView::default(),
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
            discord_id: prefs.discord_id.clone(),
            cover_cache_bytes: 0,
            about_viz: AboutViz::default(),
            auto_fetch_lyrics: prefs.auto_fetch_lyrics,
            icon_style: prefs.icon_style.clone(),
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
            live_title: None,
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
            left_list_scroll: 0,
            viewport_items: 20,
            transparent_bg: prefs.transparent_bg,
            transparent_pickers: prefs.transparent_pickers,
            reactive_theme: prefs.reactive_theme,
            reactive_theme_intensity: prefs.reactive_theme_intensity,
            reactive_palette: None,
            reactive_gen: None,
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
            idle_reset: false,
            link_up: true,
            prev_track_id: None,
            full_sync: false,
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
            live_query: None,
            live_dests: std::collections::HashSet::new(),
            live_uri: None,
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
                row: None,
                kind: LyricsKind::None,
            },
            zen: false,
            zen_surface: ZenSurface::NowPlaying,
            show_health_panel: false,
            report_health: false,
            health_report: None,
            hide_help_bar: true,
            hide_footer: false,
            left_pane_lists: left_pane_defaults(),
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

    /// Whether Zen claims `key` for itself instead of letting the normal
    /// dispatch have it.
    ///
    /// Zen used to swallow everything it did not name, which left the queue,
    /// search, the command palette and the transport unreachable from a
    /// fullscreen view — the only way out was z/Esc/q. Now everything else
    /// dispatches normally and these three are the exceptions:
    ///
    /// * `q` is `Quit` everywhere else. From a fullscreen view, with the
    ///   library and the footer both off screen, that would kill playback from
    ///   an overlay the user cannot see the rest of the app in.
    /// * `Tab` / `BackTab` cycle Zen's surface. Zen is one surface, so there is
    ///   no pane to focus, and cycling is the only thing the key could
    ///   usefully do — the bracket pair that focuses panes elsewhere has
    ///   nothing to focus here.
    /// * `Esc` closes a picker when one is open and is already handled above
    ///   this point, so it is listed only for the case where none is.
    ///
    /// `z` and `Space` are deliberately absent: they are `ToggleZen` and
    /// `PlayPause`, which already do what Zen wanted them to do.
    pub(crate) fn zen_owns(&self, key: event::KeyEvent) -> bool {
        matches!(
            key.code,
            event::KeyCode::Esc
                | event::KeyCode::Char('q')
                | event::KeyCode::Tab
                | event::KeyCode::BackTab
        )
    }

    /// The keys Zen handles: exit, cycle the surface, and play/pause.
    ///
    /// Nothing is swallowed here. Whatever Zen does not claim is dispatched as
    /// normal, which is the point — see [`Self::zen_owns`].
    fn zen_key(&mut self, key: event::KeyEvent) {
        match key.code {
            event::KeyCode::Esc | event::KeyCode::Char('q') => {
                // Both, not just `zen`. The fullscreen view is `zen ||
                // daydreaming`, so clearing one of them can leave the other up
                // -- and the idle timer would clear `daydreaming` a frame later
                // anyway, so leaving it to that made the exit depend on when
                // the next tick happened to run.
                self.zen = false;
                self.daydreaming = false;
                self.set_last_action("Leave Zen Mode", &key);
            }
            event::KeyCode::Tab => {
                self.zen_surface = self.zen_surface.next();
                self.zen_fetch_cover();
                self.set_last_action("Zen: Next Surface", &key);
            }
            event::KeyCode::BackTab => {
                self.zen_surface = self.zen_surface.prev();
                self.zen_fetch_cover();
                self.set_last_action("Zen: Prev Surface", &key);
            }
            _ => {}
        }
    }

    /// Ask for the now-playing cover when a Zen surface switch lands on it.
    ///
    /// The other two Zen surfaces draw no artwork, so this is the one switch
    /// that can need bytes the fetch has not asked for yet.
    fn zen_fetch_cover(&mut self) {
        if self.zen_surface == ZenSurface::NowPlaying && self.np_cover.image.is_none() {
            self.fetch_np_cover();
        }
    }

    /// Filtered tracks for the current library view, respecting search query, browse_detail, and category.
    /// Selection index for the currently active library list (per-category,
    /// see the `scroll_offset` field).
    pub fn list_pos(&self) -> usize {
        self.scroll_offset[self.view_slot()]
    }

    /// Set the selection index for the currently active library list.
    pub fn set_list_pos(&mut self, v: usize) {
        let slot = self.view_slot();
        self.scroll_offset[slot] = v;
    }

    /// Step the filter of whatever view is on screen: the Library grouping, or
    /// the source filter of the open picker.
    ///
    /// One entry point for `[` and `]` because they are the same question in
    /// every context — "show me the next way of slicing this" — and the picker
    /// arms and the library arm answering it separately is how the two drifted
    /// onto different keys in the first place.
    pub(crate) fn cycle_view_filter(&mut self, back: bool) {
        let Some(top) = self.pickers.top_mut() else {
            // No picker open: whichever of the two grouped views is on screen.
            // The other categories are flat lists with nothing to slice, so the
            // pair does nothing there rather than moving the category.
            match self.library_category {
                LIB_ALL => self.cycle_library_filter(back),
                LIB_PLAYLISTS => self.cycle_playlist_group(back),
                _ => {}
            }
            return;
        };
        match top.id {
            // The search pickers share one filter model.
            PickerId::SearchLibrary | PickerId::SpotifySearch => {
                top.source = if back {
                    top.source.prev()
                } else {
                    top.source.next()
                };
                top.selected = 0;
                top.viewport_offset = 0;
                // Every cover the old filter's rows had asked for is now
                // describing rows that are gone: the preview strip, the artist
                // card and the Spotify preview all have to start over.
                self.picker_preview_cover = None;
                self.picker_preview_stateful = None;
                self.picker_slot.clear();
                self.artist_cover = None;
                self.artist_cover_stateful = None;
                self.artist_slot.clear();
                self.spotify.preview_fetch.clear();
            }
            PickerId::Radio => {
                self.radio.filter = if back {
                    self.radio.filter.prev()
                } else {
                    self.radio.filter.next()
                };
            }
            // A picker with one kind of row has no filter to step.
            _ => {}
        }
    }

    /// Switch the Library view's grouping, keeping the drill-down and the
    /// selection coherent.
    ///
    /// Leaving a grouped list drops its detail: the detail is that group's
    /// name, and on the next filter it names nothing. Entering one puts the
    /// cursor back at the top, because row 3 of Albums is not row 3 of Artists.
    pub(crate) fn cycle_library_filter(&mut self, back: bool) {
        self.library_filter = if back {
            self.library_filter.prev()
        } else {
            self.library_filter.next()
        };
        if self.browse_detail.is_some() {
            self.browse_detail = None;
            self.browse_title = None;
        }
        self.dismiss_track_popup();
        self.set_list_pos(0);
        self.data_dirty = true;
        self.last_action_name = Some((
            format!("Library: {}", self.library_filter.label()),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    /// Whether the highlighted row names a group rather than a track or a
    /// playlist.
    pub fn playlist_row(&self) -> bool {
        self.library_category == LIB_PLAYLISTS && !self.playlist_open
    }

    /// Switch the Playlists view's group, by the same rules as
    /// [`Self::cycle_library_filter`]: leaving a drill-in drops it, because a
    /// playlist name names nothing in a history list.
    ///
    /// Fetching the incoming group is the part that is easy to forget and was
    /// free before: as a category it was reached by *entering* the category, and
    /// entering is what triggered the fetch. Now the four groups are one row in
    /// the left pane, so switching group is the moment the data is first needed.
    pub(crate) fn cycle_playlist_group(&mut self, back: bool) {
        self.playlist_group = if back {
            self.playlist_group.prev()
        } else {
            self.playlist_group.next()
        };
        if self.browse_detail.is_some() {
            self.browse_detail = None;
            self.browse_title = None;
        }
        self.set_list_pos(0);
        self.fetch_playlist_group();
        self.data_dirty = true;
        self.last_action_name = Some((
            format!("Playlists: {}", self.playlist_group.label()),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    /// Whether the highlighted row names a group rather than a track.
    ///
    /// An album, an artist or a genre: every Library filter but Tracks, and the
    /// one question the grouped lists all answer the same way — the row is a
    /// name, and the actions on it are about the tracks behind it.
    pub fn group_row(&self) -> bool {
        self.library_category == LIB_ALL && !matches!(self.library_filter, LibraryFilter::Tracks)
    }

    /// The rows of the Library view's active filter, in display order.
    ///
    /// One accessor because four arms of the renderer, the counts, the stats
    /// line and the drill-down all need the same list, and the filter is the
    /// only thing that decides which one it is.
    pub fn library_groups(&self) -> Vec<(String, usize)> {
        self.library_groups_of(self.library_filter)
    }

    /// The grouped rows of one filter, whatever filter is on screen.
    pub fn library_groups_of(&self, filter: LibraryFilter) -> Vec<(String, usize)> {
        match filter {
            LibraryFilter::Tracks => Vec::new(),
            LibraryFilter::Albums => self.unique_albums(),
            LibraryFilter::Artists => self.unique_artists(),
            LibraryFilter::Genres => self.unique_genres(),
            LibraryFilter::Folders => self.unique_folders(),
        }
    }

    /// Which `scroll_offset` slot the active view reads and writes.
    ///
    /// The Library view's four filters are views in their own right as far as
    /// the cursor is concerned, so they take the slots past the end of the
    /// view table.
    fn view_slot(&self) -> usize {
        if self.library_category == LIB_ALL {
            LIB_CATEGORIES.len() + self.library_filter as usize
        } else {
            self.library_category.min(LIB_CATEGORIES.len() - 1)
        }
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
            PickerId::SpotifyDest => self.live_dests_rows().saturating_sub(1),
            PickerId::YTSearch => self.yt_results_cache.len().saturating_sub(1),
            PickerId::SearchLibrary => self.search_library_picks().len().saturating_sub(1),
            PickerId::Libraries => self.filtered_library_indices().len().saturating_sub(1),
            PickerId::SpotifySearch => self.spot_picks().len().saturating_sub(1),
            PickerId::Equalizer => EQ_PRESETS.len().saturating_sub(1),
            PickerId::SleepTimer => 8,
            PickerId::Crossfade => 5,
            // Read-only: nothing to select, so no cursor to move.
            PickerId::TrackInfo => 0,
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
            PickerId::CommandPalette => CommandPalette::commands()
                .iter()
                .filter(|c| fuzzy_match(&query, c.label) || fuzzy_match(&query, c.keys))
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
                if live > 0 {
                    live
                } else {
                    self.queue.cache.len()
                }
            }
            PickerId::SpotifyDest => self.live_dests_rows().saturating_sub(1),
            PickerId::YTSearch => self.yt_results_cache.len(),
            PickerId::SearchLibrary => self.search_library_picks().len(),
            PickerId::Libraries => self.filtered_library_indices().len(),
            PickerId::SpotifySearch => self.spot_picks().len(),
            PickerId::Equalizer => EQ_PRESETS.len(),
            PickerId::SleepTimer => 9,
            PickerId::Crossfade => 6,
            PickerId::TrackInfo => 0,
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
            PickerId::CommandPalette => CommandPalette::commands()
                .iter()
                .filter(|c| fuzzy_match(&query, c.label) || fuzzy_match(&query, c.keys))
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
    ///
    /// Every user action passes through here, which is what makes it the one
    /// place the idle clock can be stamped: three separate arms would each have
    /// to remember, and the first one added later would be the one that broke
    /// daydreaming. Daemon events deliberately do not count — playback advances
    /// on its own, and a client that never stops receiving them would never
    /// go idle.
    async fn handle_terminal_event(&mut self, event: event::Event) -> bool {
        match event {
            event::Event::Key(key) => {
                self.last_input = std::time::Instant::now();
                if key.kind == KeyEventKind::Press
                    && (!self.handle_key(key).await || self.pending_quit)
                {
                    return false;
                }
            }
            event::Event::Paste(text) => {
                self.last_input = std::time::Instant::now();
                self.handle_paste(&text).await;
            }
            event::Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.last_input = std::time::Instant::now();
                    let key = event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
                    self.handle_key(key).await;
                }
                MouseEventKind::ScrollDown => {
                    self.last_input = std::time::Instant::now();
                    let key = event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
                    self.handle_key(key).await;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.last_input = std::time::Instant::now();
                    self.handle_click(mouse.column, mouse.row).await;
                }
                _ => {}
            },
            // A resize must not wait for the next tick to be noticed. The frame
            // loop reads the terminal size when it draws, so a draw issued
            // between the resize and the read compares a fresh width against the
            // buffer ratatui still holds at the old one — and that mismatch is a
            // panic in its differ, not a redraw. Clearing here drops the stale
            // buffer, so the next draw diffs against an empty one.
            event::Event::Resize(_, _) => self.resized = true,
            event::Event::FocusGained | event::Event::FocusLost => {}
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
                PickerId::SpotifyDest => {
                    self.live_dests.clear();
                    self.live_query = None;
                    self.live_uri = None;
                }
                _ => {}
            }
        }
        self.pickers.close_top();
    }

    /// Value of `field`, or `None` when it is not currently focusable.
    pub(crate) fn field_value(&self, field: &ClipField) -> Option<String> {
        Some(match field {
            ClipField::LastfmKey => self.setup.lastfm_api_key.clone(),
            ClipField::DiscordId => self.setup.discord_input.clone(),
            ClipField::YoutubeCookie => self.setup.youtube_cookie_input.clone(),
            ClipField::PodcastUrl => self.pickers.top()?.query.clone(),
            ClipField::StreamUrl => self.pickers.top()?.query.clone(),
            ClipField::SleepMinutes => self.sleep_timer.input_buf.clone(),
            ClipField::LastfmSecret => self.setup.lastfm_api_secret.clone(),
        })
    }

    /// The form field `Ctrl+V` and `Ctrl+X` apply to, if a form is open.
    ///
    /// Only the forms with a typed credential or a pasted URL qualify. A
    /// search box is not one: its query is already editable and a paste into it
    /// arrives as a bracketed-paste event, which is the terminal's own path and
    /// needs no clipboard tool.
    pub(crate) fn clipboard_field(&self) -> Option<(ClipField, String)> {
        let id = self.pickers.top()?.id;
        let field = match id {
            PickerId::LastfmAuth => match self.setup.lastfm_focus {
                0 => ClipField::LastfmKey,
                _ => ClipField::LastfmSecret,
            },
            PickerId::DiscordSetup => ClipField::DiscordId,
            PickerId::YoutubeSetup => ClipField::YoutubeCookie,
            PickerId::PodcastSubscribe => ClipField::PodcastUrl,
            PickerId::LoadStream => ClipField::StreamUrl,
            PickerId::SleepTimer => ClipField::SleepMinutes,
            _ => return None,
        };
        Some((field.clone(), self.field_value(&field)?))
    }

    /// Append pasted text to a field, keeping each form's own rule about what a
    /// valid character is.
    fn apply_paste(&mut self, field: ClipField, text: &str) {
        match field {
            ClipField::LastfmKey => self.setup.lastfm_api_key.push_str(text),
            ClipField::LastfmSecret => self.setup.lastfm_api_secret.push_str(text),
            ClipField::DiscordId => {
                // A Discord application id is digits, so a paste carrying
                // anything else is rejected rather than stored: the form would
                // otherwise accept it a character at a time and only complain
                // on Enter, by which point the id looks plausible.
                if text.chars().all(|c| c.is_ascii_digit()) {
                    self.setup.discord_input.push_str(text);
                }
            }
            // A cookie file path or a feed URL is one line, so a multi-line
            // paste is flattened rather than silently truncated mid-path.
            ClipField::YoutubeCookie => self.setup.youtube_cookie_input.push_str(text.trim()),
            ClipField::PodcastUrl | ClipField::StreamUrl => {
                if let Some(top) = self.pickers.top_mut() {
                    top.query.push_str(text.trim());
                }
            }
            ClipField::SleepMinutes => self.sleep_timer.input_buf.push_str(text.trim()),
        }
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
                    if self.spotify.oauth_field == 0 {
                        self.spotify.oauth_client_id.push_str(text);
                    } else {
                        self.spotify.oauth_port.push_str(text);
                    }
                    self.spotify.oauth_form_error = None;
                }
                PickerId::EditMetadata => {
                    self.metadata.fields[self.metadata.field_idx].push_str(text);
                }
                PickerId::PlaylistSelect if self.playlist_creating => {
                    top.query.push_str(text);
                }
                // A bracketed paste from the terminal reaches the form fields
                // directly, which is the path that works without any clipboard
                // tool at all — and the reason Ctrl+V below is a convenience
                // rather than the only way in.
                PickerId::LastfmAuth => {
                    let f = match self.setup.lastfm_focus {
                        0 => ClipField::LastfmKey,
                        _ => ClipField::LastfmSecret,
                    };
                    self.apply_paste(f, text);
                }
                PickerId::DiscordSetup => self.apply_paste(ClipField::DiscordId, text),
                PickerId::YoutubeSetup => self.apply_paste(ClipField::YoutubeCookie, text),
                PickerId::PodcastSubscribe | PickerId::LoadStream => {
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.push_str(text.trim());
                    }
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

pub mod cmd;
pub mod cover;
pub mod keys;
pub mod lyrics;
pub mod notify;
pub mod prefs;
pub mod run;
pub mod search;
pub mod settings_keys;
pub mod state;
pub mod theme;

// Per-provider actions live under `providers`; re-exported here so the glob
// import every sibling module relies on keeps resolving them.
pub use crate::providers::charts::app as charts;
pub use crate::providers::lastfm::app as lastfm;
pub use crate::providers::podcast::app as podcast;
pub use crate::providers::radio::app as radio;
pub use crate::providers::spotify::app as spotify;

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
