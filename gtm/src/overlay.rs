//! Overlay system — floating panels accessible via Alt+key from any tab.
//!
//! Each overlay is a self-contained UI module that renders on top of the
//! current tab content with a semi-transparent background.

use gtm_core::state::DaemonState;
use gtm_core::track::{Playlist, TrackInfo, YTSearchResult};

/// Every overlay variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayId {
    Queue,
    YTSearch,
    SearchLibrary,
    #[allow(dead_code)]
    SpotifySearch,
    #[allow(dead_code)]
    Equalizer,
    #[allow(dead_code)]
    CommandPalette,
    About,
    SleepTimer,
    #[allow(dead_code)]
    ThemePicker,
    VolumeConfirm,
}

/// Active overlay instance — state + metadata.
pub struct Overlay {
    pub id: OverlayId,
    pub query: String,
    pub selected: usize,
    #[allow(dead_code)]
    pub scroll_offset: usize,
}

impl Overlay {
    pub fn new(id: OverlayId) -> Self {
        Self {
            id,
            query: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }
}

/// Manages the stack of open overlays (LIFO).
pub struct OverlayManager {
    pub stack: Vec<Overlay>,
    #[allow(dead_code)]
    pub opacity: f64,
}

impl OverlayManager {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            opacity: 0.9,
        }
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn top(&self) -> Option<&Overlay> {
        self.stack.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Overlay> {
        self.stack.last_mut()
    }

    pub fn open(&mut self, id: OverlayId) {
        // Avoid duplicates
        if !self.stack.iter().any(|o| o.id == id) {
            self.stack.push(Overlay::new(id));
        }
    }

    pub fn close_top(&mut self) {
        self.stack.pop();
    }

    #[allow(dead_code)]
    pub fn close_all(&mut self) {
        self.stack.clear();
    }
}

/// Data that overlays need to read from App.
#[allow(dead_code)]
pub struct OverlayCtx<'a> {
    pub state: &'a DaemonState,
    pub tracks_cache: &'a [TrackInfo],
    pub queue_cache: &'a [TrackInfo],
    pub queue_cursor: usize,
    pub yt_results_cache: &'a [YTSearchResult],
    pub playlist_cache: &'a [Playlist],
    pub op: &'a OverlayManager,
}

impl OverlayId {
    pub fn title(&self) -> &'static str {
        match self {
            OverlayId::Queue => "Queue",
            OverlayId::YTSearch => "YouTube Search",
            OverlayId::SearchLibrary => "Search Library",
            OverlayId::SpotifySearch => "Spotify Search",
            OverlayId::Equalizer => "Equalizer",
            OverlayId::CommandPalette => "Command Palette",
            OverlayId::About => "About",
            OverlayId::SleepTimer => "Sleep Timer",
            OverlayId::ThemePicker => "Theme",
        OverlayId::VolumeConfirm => " Volume Warning ",
        }
    }
}
