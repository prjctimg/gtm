use crate::app::*;
use crate::ui::StatefulProtocol;

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
    /// Row in a loaded chart. A chart is not the local library, so the `Track`
    /// arm used to index `tracks_cache` by the row position — showing an
    /// unrelated local track's title and cover beside the chart, and at chart
    /// level 0/1 indexing the *sources* list into it.
    ChartTrack,
    /// Row in the Charts category's source list (level 0). A provider, not a
    /// track, and it carries no artwork, so the card describes the source.
    ChartSource,
    /// Row in a chart list (level 1): the chart itself, which has its own
    /// playlist artwork distinct from the artwork of the tracks inside it.
    Chart,
    /// Row in the left-pane Radio category. Stations are virtual `radio://`
    /// rows, never library tracks, so `Track` had nothing to describe.
    RadioStation,
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
    /// True once the post-link playlist fetch has been announced, so the
    /// "Synced N playlists" toast fires once rather than on every status
    /// event the daemon emits.
    pub sync_announced: bool,
    /// True while the OAuth browser flow is pending; the SpotifyLink picker
    /// shows a "waiting for you to finish login" state until linked.
    pub oauth_pending: bool,
    /// Authorize URL of the in-flight OAuth flow, shown in the SpotifyLink
    /// picker so the user can copy it even when no browser can be opened.
    pub oauth_url: Option<String>,
    /// Error from the most recent OAuth attempt, shown in the picker.
    pub oauth_error: Option<String>,
    /// Local redirect port for the Spotify OAuth flow (the only editable field
    /// in the link picker; the app id is fixed).
    pub oauth_port: String,
    /// Optional Web API app id entered in the link picker.
    ///
    /// Blank means "use librespot's", which is the previous behaviour and
    /// still works. A value here moves the Web API calls — search, artwork and
    /// playlist sync — into a rate-limit bucket of the user's own instead of the
    /// one every librespot install shares. It is deliberately *not* the id the
    /// app streams with: playback always registers as librespot's, because
    /// Spotify Connect rejects a self-registered app.
    pub oauth_client_id: String,
    /// Which field the link picker is editing: 0 = client id, 1 = port.
    pub oauth_field: usize,
    /// Local validation error on the link form, e.g. a client id pasted into
    /// the port box. Kept apart from [`Self::oauth_error`] because that one
    /// renders as a terminal flow-outcome view: overloading it would replace
    /// the form with the failure, leaving nothing to correct.
    pub oauth_form_error: Option<String>,
    /// The Web API app id the last link flow was started with, so the
    /// completion toast can name the app the account is now bound to. `None`
    /// means the flow went out with librespot's id.
    pub oauth_sent_id: Option<String>,
    pub search_debounce: Option<std::time::Instant>,
    pub web_seq: u64,
    /// Cover art for the SpotifySearch picker preview window, fetched from the
    /// album-cover URL of the highlighted web result.
    pub preview_cover: Option<Vec<u8>>,
    pub preview_cover_stateful: Option<StatefulProtocol>,
    pub preview_fetch: FetchSlot<String>,
    /// Album art already fetched this session, keyed by image URL. Several
    /// search hits share an album, and moving between them (or back after a
    /// query change) would otherwise blank the preview and refetch bytes we
    /// already hold.
    pub preview_cache: std::collections::HashMap<String, Vec<u8>>,
    /// URL whose bytes are currently published in `preview_cover`.
    /// `update_spot_preview` runs once per rendered frame, so without this the
    /// cache path would re-decode the same image every frame and make the
    /// preview flicker instead of holding still.
    pub preview_shown: Option<String>,
    /// URLs the search preview has already been refused, with the instant it
    /// may be retried.
    ///
    /// A miss caches nothing, so without this the preview re-requested the same
    /// album every rendered frame — `update_spot_preview` is called from the
    /// render path — which is a Spotify CDN request carrying a bearer token at
    /// up to frame rate. Mirrors the throttle the queue strip already uses.
    pub preview_fail_until: std::collections::HashMap<String, std::time::Instant>,
    /// Cover art for the highlighted row of the playlist drill-down.
    ///
    /// Modelled on the search preview rather than sharing it: the two views are
    /// never open at once, and a shared slot would make whichever rendered last
    /// clear the other's image. The daemon's background prefetch has already put
    /// the playlist's covers on disk by the time this is asked for, so the
    /// fetch is a local read.
    pub row_cover: Option<Vec<u8>>,
    pub row_cover_stateful: Option<StatefulProtocol>,
    pub row_fetch: FetchSlot<String>,
    /// Playlist row whose cover is on screen, so moving the cursor only
    /// re-decodes when it lands on a different track.
    pub row_cover_index: Option<usize>,
    pub row_shown: Option<String>,
    /// Playlist whose covers have already been swept into the daemon's cache.
    /// The sweep runs from the cursor-move path, so without this a thousand
    /// track playlist would be re-requested on every keypress.
    pub prefetched_for: Option<String>,
    /// Cover art of the highlighted *playlist*, shown in the left pane's info
    /// slot. Separate from `row_cover` because the two views are never open at
    /// once and sharing one slot would make whichever rendered last blank the
    /// other.
    pub list_cover: Option<Vec<u8>>,
    pub list_cover_stateful: Option<StatefulProtocol>,
    pub list_fetch: FetchSlot<String>,
    pub list_shown: Option<String>,
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

/// One decoded cover, kept beside its bytes.
///
/// `Render::cover` wants a protocol, not bytes, and the protocol is the
/// expensive half: decoding an image on every frame is what makes a preview
/// flicker rather than hold still, so a cell decodes once and keeps the result
/// for as long as it is on screen.
pub struct GridCell {
    pub bytes: Vec<u8>,
    pub proto: Option<StatefulProtocol>,
}

impl GridCell {
    /// Decode the bytes once. Called on the reply, not on the draw.
    ///
    /// The picker is passed in rather than reached for because it lives on the
    /// now-playing cover state, which the caller is already borrowing from: a
    /// cell cannot ask for it without a second mutable borrow of the app.
    pub fn new(bytes: Vec<u8>, picker: Option<&Picker>) -> Self {
        let proto = picker.and_then(|p| {
            image::load_from_memory(&bytes)
                .ok()
                .map(|img| p.new_resize_protocol(img))
        });
        Self { bytes, proto }
    }
}

/// The cover-grid view of the album, artist and genre lists.
///
/// The lists themselves are text rows because a thousand albums are a thousand
/// names; a grid is how you *browse* a shelf of covers, and there is no second
/// screen to browse them on. So it is a mode of the same list, switched with a
/// key, rather than a new view — the cursor, the selection, the drill-down and
/// the stats line are the ones the row view already has.
#[derive(Default)]
pub struct GridView {
    /// Whether the categories that have artwork draw a grid.
    pub on: bool,
    /// Cells across, and cells down, as of the last frame's pane. The cursor
    /// moves by a row of cells rather than by one, so this has to be the
    /// geometry the last paint used rather than one guessed at keypress time:
    /// a terminal resized since the last keypress has a different answer.
    pub cols: usize,
    pub rows: usize,
    /// Index of the item in the top-left cell.
    pub first: usize,
    /// Representative track behind each on-screen cell, in cell order. `None`
    /// where the item has no track behind it; those cells draw a placeholder.
    ///
    /// Only the window, not the list: resolving an item's representative track
    /// scans the library, so doing it for a thousand albums on every frame was
    /// a million string comparisons a frame for cells nobody could see.
    pub ids: Vec<Option<i64>>,
    /// Decoded artwork, keyed by track id.
    pub covers: std::collections::HashMap<i64, GridCell>,
    /// Ids already asked for, so a cell is fetched once per session rather than
    /// once per frame. A miss stays in here too: the daemon has no cover for
    /// that track, and asking again every frame would be a request per frame.
    pub asked: std::collections::HashSet<i64>,
    /// Bumped when the window or the list changes, so a reply for a cell that
    /// has scrolled away is dropped rather than painted under a different item.
    pub round: u64,
}

/// Columns of artwork in one grid cell, and the rows it takes with its label.
pub(crate) const GRID_CELL_W: u16 = 14;
pub(crate) const GRID_CELL_H: u16 = 8;

/// How the grid lays out in a given pane, and which items it shows.
///
/// Computed by the renderer and handed back to the rest of the app, because
/// every other part of the grid — the cursor's row step, which cells to fetch —
/// is a function of the pane's size and nothing else knows it.
pub struct GridPlan {
    pub first: usize,
    pub cols: usize,
    pub rows: usize,
}

impl GridView {
    /// Enforce [`GRID_CACHE_MAX`] by dropping entries until the cache fits.
    ///
    /// `HashMap` has no order, so "oldest" is not available and the entries that
    /// go are arbitrary. That is acceptable because a cover dropped while its
    /// cell is on screen is re-requested through the normal path rather than
    /// staying blank: `asked` is cleared alongside it.
    pub fn prune_hard(&mut self) {
        while self.covers.len() > GRID_CACHE_MAX {
            let Some(victim) = self.covers.keys().next().copied() else {
                break;
            };
            self.covers.remove(&victim);
            self.asked.remove(&victim);
        }
    }
}

/// How many cells' covers may be requested per frame.
///
/// The daemon serves each from its disk cache, so this is not a rate limit but
/// a bound on the work one frame can queue: a wide pane holds forty cells and
/// asking for all of them at once put forty replies in flight behind the first
/// redraw.
pub(crate) const GRID_FETCH_BATCH: usize = 6;

/// Cells retained at once.
///
/// A wide pane holds around forty, so this is roughly two screens: a scroll back
/// over anything you just looked at is a cache hit, and a session that walks a
/// thousand albums does not keep a thousand decoded images alive.
pub(crate) const GRID_CACHE_MAX: usize = 96;

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
    /// Discord application id draft for the `DiscordSetup` form.
    pub discord_input: String,
}

/// Selected row of the `gtm setup` service chooser.
pub fn setup_selection(app: &App) -> (usize, &'static str) {
    let names = ["spotify", "lastfm", "youtube", "discord"];
    let sel = app.setup.selection.min(3);
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
    /// Public-directory results for the picker's search box.
    pub results: Vec<PodcastResult>,
    /// A search is in flight, so the picker can say so rather than showing an
    /// empty list that reads as "nothing matched".
    pub searching: bool,
    /// When the picker's query should be sent to the directory. Settled rather
    /// than immediate: a third-party query should not fire per keystroke.
    pub search_deadline: Option<std::time::Instant>,
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

    pub fn prev(self) -> Self {
        match self {
            Self::Name => Self::Rating,
            Self::Tags => Self::Name,
            Self::Country => Self::Tags,
            Self::Rating => Self::Country,
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
    /// Cover art for the queue picker preview strip: fetched for the row under
    /// the highlight, including locally-inserted (`id == 0`) entries, which is
    /// why the guard is keyed on `path` rather than on a library id.
    pub preview_cover: Option<Vec<u8>>,
    pub preview_cover_stateful: Option<StatefulProtocol>,
    pub preview_slot: FetchSlot<String>,
    pub preview_fail_until: Option<(String, std::time::Instant)>,
}

/// Lyrics pane UI state, grouped under `App::lyrics`.
/// Which Zen-mode surface is shown. Only one is visible at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZenSurface {
    /// Now playing: enlarged cover art, the track title and artist on their
    /// own lines, the current lyric line under the art, and the progress bar
    /// below all of it.
    NowPlaying,
    /// Full-screen lyrics for the track on air, sharing the body rendering of
    /// the normal lyrics pane.
    Lyrics,
    /// Full-screen audio visualizer.
    Visualizer,
}

impl ZenSurface {
    pub(crate) fn next(self) -> ZenSurface {
        match self {
            ZenSurface::NowPlaying => ZenSurface::Lyrics,
            ZenSurface::Lyrics => ZenSurface::Visualizer,
            ZenSurface::Visualizer => ZenSurface::NowPlaying,
        }
    }

    pub(crate) fn prev(self) -> ZenSurface {
        match self {
            ZenSurface::NowPlaying => ZenSurface::Visualizer,
            ZenSurface::Visualizer => ZenSurface::Lyrics,
            ZenSurface::Lyrics => ZenSurface::NowPlaying,
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
    /// Playlist row whose lyrics are on screen. Moving the cursor re-fetches
    /// only when it lands on a different track, so holding an arrow key does
    /// not re-request the same row once per frame.
    pub row: Option<usize>,
    /// What `current` is holding.
    ///
    /// A lyrics reply that finds nothing used to be written into `current` as a
    /// "No lyrics found" line, which is indistinguishable from a real result.
    /// A podcast transcript is fetched by episode name, not by the playing
    /// track, and is still on screen when the track changes — so the next
    /// track's lyrics fetch either replaced it or, coming back empty, replaced
    /// it with "No lyrics found". The kind makes the two tellable apart.
    pub kind: LyricsKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LyricsKind {
    /// Nothing on screen.
    #[default]
    None,
    /// A track's lyrics from the lyrics manager.
    Track,
    /// A "No lyrics found" placeholder: a real answer, and an empty one.
    Missing,
    /// A podcast episode's transcript, requested by name and unrelated to
    /// whatever is playing.
    Transcript,
}

/// A text field a clipboard paste or copy is aimed at.
///
/// Which field is focusable differs per form, so the paste result names one of
/// these rather than reaching into a form the user has since closed.
#[derive(Clone, Debug)]
pub(crate) enum ClipField {
    LastfmKey,
    LastfmSecret,
    DiscordId,
    YoutubeCookie,
    PodcastUrl,
    StreamUrl,
    SleepMinutes,
}
