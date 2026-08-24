// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Floating picker panels for settings and search
//
// This is free software released under the GPL-3.0 license.

/// Every picker variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerId {
    Queue,
    YTSearch,
    SearchLibrary,
    SpotifySearch,
    SpotifyLink,
    Equalizer,
    CommandPalette,
    About,
    SleepTimer,
    ThemePicker,
    Help,
    PlaylistSelect,
    EditMetadata,
    Crossfade,
    VisualizerPreset,
    ProgressStyle,
    Settings,
    Notifications,
}

/// Which list a fuzzy-finder picker searches. `Tab` cycles through these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerSource {
    #[default]
    All,
    Tracks,
    Artists,
    Albums,
    Playlists,
}

impl PickerSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Tracks => "Tracks",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Tracks,
            Self::Tracks => Self::Artists,
            Self::Artists => Self::Albums,
            Self::Albums => Self::Playlists,
            Self::Playlists => Self::All,
        }
    }
}

/// Active picker instance: state + metadata.
pub struct Picker {
    pub id: PickerId,
    pub query: String,
    pub selected: usize,
    /// Top row of the visible window (1-row-at-a-time scroll model).
    /// Only advances when the selection leaves the viewport, so the
    /// list never recenters or jumps.
    pub viewport_offset: usize,
    /// List the fuzzy finder filters over (`Tab` cycles it).
    pub source: PickerSource,
}

impl Picker {
    pub fn new(id: PickerId) -> Self {
        Self {
            id,
            query: String::new(),
            selected: 0,
            viewport_offset: 0,
            source: PickerSource::default(),
        }
    }
}

/// Manages the stack of open pickers (LIFO).
pub struct PickerManager {
    pub stack: Vec<Picker>,
}

impl PickerManager {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
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
}

impl Default for PickerManager {
    fn default() -> Self {
        Self::new()
    }
}
