// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Picker system — floating panels accessible via Alt+key from any tab
//
// This is free software released under the GPL-3.0 license.

//! Picker system — floating panels accessible via Alt+key from any tab.
//!
//! Each picker is a self-contained UI module that renders on top of the
//! current tab content with a semi-transparent background.

use gtm_core::state::DaemonState;
use gtm_core::track::{Playlist, TrackInfo, YTSearchResult};

/// Every picker variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerId {
    Queue,
    YTSearch,
    SearchLibrary,
    SpotifySearch,
    Equalizer,
    CommandPalette,
    About,
    SleepTimer,
    ThemePicker,
    SoundEffects,
    Help,
    PlaylistSelect,
    EditMetadata,
}

/// Active picker instance — state + metadata.
pub struct Picker {
    pub id: PickerId,
    pub query: String,
    pub selected: usize,
    #[allow(dead_code)]
    pub scroll_offset: usize,
}

impl Picker {
    pub fn new(id: PickerId) -> Self {
        Self {
            id,
            query: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }
}

/// Manages the stack of open pickers (LIFO).
pub struct PickerManager {
    pub stack: Vec<Picker>,
    #[allow(dead_code)]
    pub opacity: f64,
}

impl PickerManager {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            opacity: 0.9,
        }
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn top(&self) -> Option<&Picker> {
        self.stack.last()
    }

    pub fn top_mut(&mut self) -> Option<&mut Picker> {
        self.stack.last_mut()
    }

    pub fn open(&mut self, id: PickerId) {
        // Avoid duplicates
        if !self.stack.iter().any(|o| o.id == id) {
            self.stack.push(Picker::new(id));
        }
    }

    pub fn open_with_selection(&mut self, id: PickerId, selected: usize) {
        if !self.stack.iter().any(|o| o.id == id) {
            let mut picker = Picker::new(id);
            picker.selected = selected;
            self.stack.push(picker);
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

/// Data that pickers need to read from App.
#[allow(dead_code)]
pub struct PickerCtx<'a> {
    pub state: &'a DaemonState,
    pub tracks_cache: &'a [TrackInfo],
    pub queue_cache: &'a [TrackInfo],
    pub queue_cursor: usize,
    pub yt_results_cache: &'a [YTSearchResult],
    pub playlist_cache: &'a [Playlist],
    pub op: &'a PickerManager,
}

impl PickerId {
    #[allow(dead_code)]
    pub fn title(&self) -> &'static str {
        match self {
            PickerId::Queue => "Queue",
            PickerId::YTSearch => "YouTube Search",
            PickerId::SearchLibrary => "Search Library",
            PickerId::SpotifySearch => "Spotify Search",
            PickerId::Equalizer => "Equalizer",
            PickerId::CommandPalette => "Command Palette",
            PickerId::About => "About",
            PickerId::SleepTimer => "Sleep Timer",
            PickerId::ThemePicker => "Theme",
            PickerId::SoundEffects => "Sound Effects",
            PickerId::Help => "Help",
            PickerId::PlaylistSelect => "Add to Playlist",
            PickerId::EditMetadata => "Edit Metadata",
        }
    }
}
