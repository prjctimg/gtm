use crate::app::*;

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
    pub rate_bps: Option<f64>,
    pub eta_secs: Option<u64>,
    pub updated_at: std::time::Instant,
}

pub struct UpNextNotif {
    pub track: TrackInfo,
    pub cover: Option<Vec<u8>>,
    pub cover_stateful: Option<StatefulProtocol>,
    pub started_at: std::time::Instant,
    pub total_secs: f64,
    /// In-flight cover request for `track`, so a stale reply is dropped and
    /// a repeated request for the same track is not re-issued.
    pub cover_fetch: FetchSlot<i64>,
}

/// Fuzzy subsequence match: every byte of `q` appears in order in `hay`.
/// Used by every fuzzy-finder (library search, themes, command palette) so
/// filtering behaviour is identical across pickers.
/// Spotify search rows matching the picker's `PickerSource` filter. Both the
/// item count and the renderer read through this so they cannot disagree.
impl App {
    pub fn spot_picks(&self) -> Vec<usize> {
        let src = self.pickers.top().map_or(PickerSource::All, |o| o.source);
        self.spotify
            .search_results
            .iter()
            .enumerate()
            .filter(|(_, (_, _, t))| match src {
                PickerSource::All => true,
                PickerSource::Tracks => t.kind.is_none(),
                PickerSource::Artists => t.kind == Some(SpotifySearchKind::Artist),
                PickerSource::Albums => t.kind == Some(SpotifySearchKind::Album),
                PickerSource::Playlists => t.kind == Some(SpotifySearchKind::Playlist),
                PickerSource::Radio => false,
            })
            .map(|(i, _)| i)
            .collect()
    }
}

pub(crate) fn fuzzy_match(q: &str, hay: &str) -> bool {
    let q = q.to_lowercase();
    if q.is_empty() {
        return true;
    }
    let mut qi = 0usize;
    for ch in hay.to_lowercase().chars() {
        if qi < q.len() && ch == q.as_bytes()[qi] as char {
            qi += 1;
        }
    }
    qi == q.len()
}

/// One row in the SearchLibrary fuzzy-finder, resolved from a `PickerSource`.
#[derive(Debug, Clone)]
pub enum LibraryPick {
    Track(usize),
    Artist(String),
    Album(String),
    Playlist(usize),
    Radio(usize),
}

pub struct SleepTimerState {
    pub remaining: Option<u64>,
    pub minutes: u32,
    pub input_mode: bool,
    pub input_buf: String,
    /// "End playback immediately when the time is up" checkbox (default
    /// checked). Off defers the stop to the natural end of the current
    /// finite track; radio/endless streams always stop immediately.
    pub stop_immediately: bool,
    /// Focused row within the picker for Up/Down option navigation.
    pub focus: usize,
}

pub struct MetadataEditState {
    /// Tracks being edited. Usually one, but an album/artist row opens the
    /// editor with every cached track in that album/artist so the same field
    /// edits apply to the whole batch (cover sync only uses `first()`).
    pub edit_track_ids: Vec<i64>,
    pub fields: [String; 7],
    pub field_idx: usize,
    pub cover: Option<Vec<u8>>,
    pub cover_stateful: Option<StatefulProtocol>,
    pub cover_dirty: bool,
    /// In-flight cover request for the first edited track.
    pub cover_fetch: FetchSlot<i64>,
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

#[derive(Debug, Clone)]
pub enum PromptType {
    DeleteTrack(i64),
    DeletePlaylist(i64),
    MultiselectDelete(Vec<i64>),
    MultiselectAddToQueue,
    MultiselectAddToPlaylist,
    /// Remove a custom station from `radios.toml` by name.
    RemoveCustomRadio(String),
    None,
}

/// Guard for a cover/preview fetch slot: the target id (or URL) the
/// in-flight response belongs to, plus a generation used to drop stale
/// replies. Replaces duplicated `last_*_fetch_id`/`version` bookkeeping pairs.
///
/// One slot per resource, so at most one fetch per resource is ever in
/// flight: a new target bumps the generation, which makes the previous
/// response a no-op on arrival, and `matches` lets a repeated request for
/// the same target be skipped entirely.
pub struct FetchSlot<T> {
    pub id: Option<T>,
    pub version: Option<u64>,
}

impl<T> Default for FetchSlot<T> {
    fn default() -> Self {
        FetchSlot {
            id: None,
            version: None,
        }
    }
}

impl<T: PartialEq> FetchSlot<T> {
    /// True when a request for `id` is already in flight, so the caller can
    /// skip issuing a duplicate fetch for the same target.
    pub fn pending(&self, id: &T) -> bool {
        self.version.is_some() && self.id.as_ref() == Some(id)
    }

    /// Claim the slot for `id` at generation `version`.
    pub fn claim(&mut self, id: T, version: u64) {
        self.id = Some(id);
        self.version = Some(version);
    }

    /// True when a reply tagged `version` still belongs to this request.
    pub fn matches(&self, version: u64) -> bool {
        self.version == Some(version)
    }

    /// Release the slot so a later visit can retry.
    pub fn clear(&mut self) {
        self.id = None;
        self.version = None;
    }
}

/// Spotify search/link UI state, grouped under `App::spotify`.
pub struct SpotifyView {
    pub status: Option<SpotifyStatus>,
    pub playlists: Vec<SpotifyPlaylist>,
    pub playlist_tracks_cache: Vec<SpotifyTrack>,
    pub search_results: Vec<(String, String, SpotifyTrack)>,
    pub link_input: String,
    /// True while a Spotify web search is in flight, so the picker can show a
    /// spinner instead of "No results found".
    pub search_loading: bool,
    /// True while a playlist sync spawned by the TUI is in flight; guards
    /// against duplicate auto-syncs stacking up.
    pub sync_pending: bool,
    /// True once a TUI-side sync completed successfully. The pane only
    /// auto-syncs once per process (when the cache is empty on open); manual
    /// Settings -> Sync always works regardless.
    pub synced_once: bool,
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
    pub preview_fetch: FetchSlot<String>,
}

/// Top Charts picker state, grouped under `App::charts`.
#[derive(Default)]
pub struct ChartsView {
    pub sources: Vec<crate::shared::chart::ChartSource>,
    pub charts: Vec<crate::shared::chart::ChartPlaylist>,
    pub chart_tracks: Vec<crate::shared::chart::ChartTrack>,
    pub selected_source: Option<usize>,
    pub selected_chart: Option<usize>,
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
    /// YouTube cookie-file draft for the `YoutubeSetup` form.
    pub youtube_cookie_input: String,
}

/// Selected row of the `gtm setup` service chooser.
pub fn setup_selection(app: &App) -> (usize, &'static str) {
    let names = ["spotify", "lastfm", "youtube"];
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

/// Which list the unified Radio picker is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioSection {
    /// Merged root list: Saved / Top / Tags / Countries with headers.
    #[default]
    Root,
    /// Stations of the tag/country in `RadioView::browse_topic`.
    Stations,
    /// Directory search results.
    Results,
}

/// Which field the radio picker's query filters on. `Tab` cycles through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioFilter {
    #[default]
    Name,
    Tags,
    Country,
    Rating,
}

impl RadioFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Tags => "Tags",
            Self::Country => "Country",
            Self::Rating => "Rating",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Tags,
            Self::Tags => Self::Country,
            Self::Country => Self::Rating,
            Self::Rating => Self::Name,
        }
    }
}

/// A selectable row in the unified Radio picker, mirroring how the
/// `SearchLibrary` picker builds `LibraryPick` rows. Build once per frame
/// (and per key event) from the active section and query filter, so the
/// picker's selection index maps 1:1 into the rendered row list.
#[derive(Debug, Clone)]
pub enum RadioPick {
    /// Section divider (label) in the merged Root view. Selecting it does nothing.
    Header(&'static str),
    /// Index into `radio.custom` (a saved station).
    Custom(usize),
    /// Index into the station list of the current section (`top` in the Root
    /// view, `browse_stations`, or `search`).
    Station(usize),
    /// Index into `radio.browse_tags` (drill into its stations).
    Tag(usize),
    /// Index into `radio.browse_countries` (drill into its stations).
    Country(usize),
}

/// How the `Stations` section was entered, so `r` can re-fetch the same
/// directory query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioBrowseBy {
    #[default]
    Tag,
    Country,
}

/// Radio Browser picker state, grouped under `App::radio`.
#[derive(Default)]
pub struct RadioView {
    pub search: Vec<RadioStation>,
    pub search_pending: bool,
    pub top: Vec<RadioStation>,
    pub top_pending: bool,
    /// Tag list (Root/Tags section).
    pub browse_tags: Vec<RadioTag>,
    /// Country list (Root/Countries section).
    pub browse_countries: Vec<RadioCountry>,
    pub browse_pending: bool,
    /// Tag/country selected at Tags/Countries; stations live in
    /// `browse_stations`.
    pub browse_topic: String,
    /// How `Stations` was entered (by tag or by country).
    pub browse_by: RadioBrowseBy,
    pub browse_stations: Vec<RadioStation>,
    pub browse_stations_pending: bool,
    /// Which section of the unified picker is showing.
    pub section: RadioSection,
    /// Which field the picker query filters on.
    pub filter: RadioFilter,
    /// Custom stations from `radios.toml` (1-based index = position + 1),
    /// mirrored into the left-pane Radio category.
    pub custom: Vec<CustomRadioStation>,
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
    pub preview_slot: FetchSlot<i64>,
    pub preview_fail_until: Option<(i64, std::time::Instant)>,
}

/// Lyrics pane UI state, grouped under `App::lyrics`.
/// Which Zen-mode surface is shown. Only one is visible at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZenSurface {
    /// Enlarged cover art with the track progress centered underneath.
    Cover,
    /// Full-screen audio visualizer.
    Visualizer,
    /// Full-screen lyrics for the active track.
    Lyrics,
}

impl ZenSurface {
    pub(crate) fn next(self) -> ZenSurface {
        match self {
            ZenSurface::Cover => ZenSurface::Visualizer,
            ZenSurface::Visualizer => ZenSurface::Lyrics,
            ZenSurface::Lyrics => ZenSurface::Cover,
        }
    }

    pub(crate) fn prev(self) -> ZenSurface {
        match self {
            ZenSurface::Cover => ZenSurface::Lyrics,
            ZenSurface::Visualizer => ZenSurface::Cover,
            ZenSurface::Lyrics => ZenSurface::Visualizer,
        }
    }
}

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
    /// User-applied time offset in seconds (added to the playback position for
    /// lyric matching and timestamp display). Adjusted with `[` / `]` while the
    /// lyrics pane holds focus and reset to zero on every track change.
    pub offset_secs: f64,
}
