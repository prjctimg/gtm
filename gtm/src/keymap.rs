// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Keybinding dispatch system with context-aware matching
//
// This is free software released under the GPL-3.0 license.

//! Keybinding dispatch system with context-aware matching.
//!
//! ```text
//!  Key press
//!      │
//!      ▼
//!  ┌──────────────────────┐
//!  │ key_matches()        │  Scan binding list for matching KeyEvent
//!  │ && context matches   │  AND current KeyContext (Normal, Filter, etc.)
//!  └──────────┬───────────┘
//!             │ hit
//!             ▼
//!  ┌──────────────────────┐
//!  │ KeyboardAction enum  │  → e.g. PlayPause, NextTab, Quit, VolumeUp
//!  └──────────────────────┘
//!
//!  Contexts:
//!    Global  — active everywhere (q, space, ?)
//!    Normal  — main view mode (tab, cursor, volume)
//!    List    — when a list widget has focus (j/k, enter)
//!    Filter  — typing a search query
//!    Overlay — modal overlay is open
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gtm_core::state::Tab;

use crate::overlay::OverlayId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum KeyContext {
    Global,
    Normal,
    Filter,
    Overlay,
    List,
    MoveMode,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum KeyboardAction {
    // Tab switching
    NextTab,
    PrevTab,
    SwitchTab(Tab),

    // Cursor
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Top,
    Bottom,

    // Selection / action
    Select,
    EnqueueNext,
    EnqueueEnd,
    Delete,
    Move,

    // Filter
    EnterFilter,
    EnterCommand,
    ClearFilter,
    Confirm,

    // Playback
    PlayPause,
    Next,
    Prev,
    Stop,
    VolumeUp,
    VolumeDown,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    CycleRepeat,
    ToggleMute,
    ToggleFavourite,

    // Queue
    ClearQueue,

    // Overlay triggers
    OpenOverlay(OverlayId),

    // Navigation
    Back,
    FocusLeft,
    FocusRight,

    // Lyrics
    FetchLyrics,

    // Library motions (vim-style)
    ToggleMultiselect,
    ToggleSelectAndAdvance,
    AddToQueue,
    AddToPlaylist,
    DeleteFromList,
    JumpToStart,
    JumpToEnd,
    EditMetadata,

    // Meta
    Quit,
    QuitDaemon,
    ReloadConfig,
    ToggleHelp,
    CycleFooterPreset,
    CycleProgressStyle,
    ToggleVisualizer,
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct BoundCommand {
    pub action: KeyboardAction,
    pub contexts: Vec<KeyContext>,
    #[allow(dead_code)]
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct Keybindings {
    pub bindings: Vec<(KeyEvent, BoundCommand)>,
}

impl Keybindings {
    /// Find the first binding whose KeyEvent matches and whose contexts
    /// include the current `context`.  Returns `None` if no binding matches.
    pub fn dispatch(&self, key: KeyEvent, context: KeyContext) -> Option<KeyboardAction> {
        for (binding_key, cmd) in &self.bindings {
            if key_matches(&key, binding_key) && cmd.contexts.contains(&context) {
                return Some(cmd.action.clone());
            }
        }
        None
    }
}

fn key_matches(event: &KeyEvent, binding: &KeyEvent) -> bool {
    event.code == binding.code && event.modifiers == binding.modifiers
}

/// Parse a key name string into a KeyEvent.  Used for configurability
/// (e.g. from a config file) though currently only called from tests.
#[allow(dead_code)]
pub fn parse_keycode(name: &str) -> KeyEvent {
    match name {
        "tab" | "Tab" => KeyCode::Tab.into(),
        "backtab" | "BackTab" => KeyCode::BackTab.into(),
        "enter" | "Enter" => KeyCode::Enter.into(),
        "esc" | "Escape" => KeyCode::Esc.into(),
        "space" => KeyCode::Char(' ').into(),
        "up" | "Up" => KeyCode::Up.into(),
        "down" | "Down" => KeyCode::Down.into(),
        "left" | "Left" => KeyCode::Left.into(),
        "right" | "Right" => KeyCode::Right.into(),
        "home" | "Home" => KeyCode::Home.into(),
        "end" | "End" => KeyCode::End.into(),
        "pageup" | "PageUp" => KeyCode::PageUp.into(),
        "pagedown" | "PageDown" => KeyCode::PageDown.into(),
        "delete" | "Del" => KeyCode::Delete.into(),
        "backspace" | "BS" => KeyCode::Backspace.into(),
        "f1" => KeyCode::F(1).into(),
        "f2" => KeyCode::F(2).into(),
        "f3" => KeyCode::F(3).into(),
        "f4" => KeyCode::F(4).into(),
        "f5" => KeyCode::F(5).into(),
        "f6" => KeyCode::F(6).into(),
        "f7" => KeyCode::F(7).into(),
        "f8" => KeyCode::F(8).into(),
        "f9" => KeyCode::F(9).into(),
        "f10" => KeyCode::F(10).into(),
        "f11" => KeyCode::F(11).into(),
        "f12" => KeyCode::F(12).into(),
        "ctrl-c" | "CtrlC" => KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        "ctrl-d" | "CtrlD" => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        "ctrl-u" | "CtrlU" => KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        "ctrl-r" | "CtrlR" => KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        s if s.len() == 1 => KeyCode::Char(s.chars().next().unwrap()).into(),
        _ => KeyCode::Char('?').into(),
    }
}

/// Build the default set of key bindings.  Layered by context:
///
///   Global  — q (quit), ? (help), space (play/pause)
///   Normal  — tab switching, cursor, volume, filters, playback control
///   List    — j/k, enter, delete
///
/// Bindings are scanned in order; the first match wins.
pub fn default_keybindings() -> Keybindings {
    Keybindings {
        bindings: vec![
            // Global — Quit
            (
                KeyCode::Char('q').into(),
                BoundCommand {
                    action: KeyboardAction::Quit,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Quit",
                },
            ),
            // Global — ToggleHelp
            (
                KeyCode::Char('?').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleHelp,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Toggle help",
                },
            ),
            // Command palette (:)
            (
                KeyCode::Char(':').into(),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::CommandPalette),
                    contexts: vec![KeyContext::Normal],
                    description: "Open command palette",
                },
            ),
            // Tab switching — Tab / Shift-Tab
            (
                KeyCode::Tab.into(),
                BoundCommand {
                    action: KeyboardAction::NextTab,
                    contexts: vec![KeyContext::Normal],
                    description: "Next tab",
                },
            ),
            (
                KeyCode::BackTab.into(),
                BoundCommand {
                    action: KeyboardAction::PrevTab,
                    contexts: vec![KeyContext::Normal],
                    description: "Previous tab",
                },
            ),
            // Cursor — arrow keys
            (
                KeyCode::Up.into(),
                BoundCommand {
                    action: KeyboardAction::MoveUp,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Move up",
                },
            ),
            (
                KeyCode::Down.into(),
                BoundCommand {
                    action: KeyboardAction::MoveDown,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Move down",
                },
            ),
            // Cursor — vim-style j/k
            (
                KeyCode::Char('k').into(),
                BoundCommand {
                    action: KeyboardAction::MoveUp,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Move up",
                },
            ),
            (
                KeyCode::Char('j').into(),
                BoundCommand {
                    action: KeyboardAction::MoveDown,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Move down",
                },
            ),
            // Playback — space, n, p, Ctrl+N/P, s (stop)
            (
                KeyCode::Char(' ').into(),
                BoundCommand {
                    action: KeyboardAction::PlayPause,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Play / Pause",
                },
            ),
            (
                KeyCode::Char('n').into(),
                BoundCommand {
                    action: KeyboardAction::Next,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Next track",
                },
            ),
            (
                KeyCode::Char('p').into(),
                BoundCommand {
                    action: KeyboardAction::Prev,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Previous track",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::Next,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Next track",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::Prev,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Previous track",
                },
            ),
            (
                KeyCode::Char('s').into(),
                BoundCommand {
                    action: KeyboardAction::Stop,
                    contexts: vec![KeyContext::Normal],
                    description: "Stop playback",
                },
            ),
            // Volume — +, =, -
            (
                KeyCode::Char('+').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeUp,
                    contexts: vec![KeyContext::Normal],
                    description: "Volume up",
                },
            ),
            (
                KeyCode::Char('=').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeUp,
                    contexts: vec![KeyContext::Normal],
                    description: "Volume up",
                },
            ),
            (
                KeyCode::Char('-').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeDown,
                    contexts: vec![KeyContext::Normal],
                    description: "Volume down",
                },
            ),
            // Mute — m
            (
                KeyCode::Char('m').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMute,
                    contexts: vec![KeyContext::Normal],
                    description: "Toggle mute",
                },
            ),
            // Quit daemon — Q
            (
                KeyCode::Char('Q').into(),
                BoundCommand {
                    action: KeyboardAction::QuitDaemon,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                    description: "Quit and stop daemon",
                },
            ),
            // Favourite — F
            (
                KeyCode::Char('F').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleFavourite,
                    contexts: vec![KeyContext::Normal],
                    description: "Toggle favourite",
                },
            ),
            // Clear queue — D
            (
                KeyCode::Char('D').into(),
                BoundCommand {
                    action: KeyboardAction::ClearQueue,
                    contexts: vec![KeyContext::Normal],
                    description: "Clear queue",
                },
            ),
            // Focus navigation — [, ]
            (
                KeyCode::Char('[').into(),
                BoundCommand {
                    action: KeyboardAction::FocusLeft,
                    contexts: vec![KeyContext::Normal],
                    description: "Focus left pane",
                },
            ),
            (
                KeyCode::Char(']').into(),
                BoundCommand {
                    action: KeyboardAction::FocusRight,
                    contexts: vec![KeyContext::Normal],
                    description: "Focus right pane",
                },
            ),
            // Lyrics — l
            (
                KeyCode::Char('l').into(),
                BoundCommand {
                    action: KeyboardAction::FetchLyrics,
                    contexts: vec![KeyContext::Normal],
                    description: "Fetch lyrics",
                },
            ),
            // Repeat — r, Shift+R
            (
                KeyCode::Char('r').into(),
                BoundCommand {
                    action: KeyboardAction::CycleRepeat,
                    contexts: vec![KeyContext::Normal],
                    description: "Cycle repeat",
                },
            ),
            (
                KeyCode::Char('R').into(),
                BoundCommand {
                    action: KeyboardAction::CycleRepeat,
                    contexts: vec![KeyContext::Normal],
                    description: "Cycle repeat",
                },
            ),
            // Shuffle — Shift+S only (s is reserved for Stop)
            (
                KeyCode::Char('S').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleShuffle,
                    contexts: vec![KeyContext::Normal],
                    description: "Toggle shuffle",
                },
            ),
            // Seek — comma/period
            (
                KeyCode::Char('.').into(),
                BoundCommand {
                    action: KeyboardAction::SeekForward,
                    contexts: vec![KeyContext::Normal],
                    description: "Seek forward",
                },
            ),
            (
                KeyCode::Char(',').into(),
                BoundCommand {
                    action: KeyboardAction::SeekBackward,
                    contexts: vec![KeyContext::Normal],
                    description: "Seek backward",
                },
            ),
            // Filter mode — /, Ctrl+F
            (
                KeyCode::Char('/').into(),
                BoundCommand {
                    action: KeyboardAction::EnterFilter,
                    contexts: vec![KeyContext::Normal],
                    description: "Enter filter mode",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::EnterFilter,
                    contexts: vec![KeyContext::Normal],
                    description: "Enter filter mode",
                },
            ),
            // Select — Enter
            (
                KeyCode::Enter.into(),
                BoundCommand {
                    action: KeyboardAction::Select,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Select item",
                },
            ),
            // Back — Backspace (go back in library drill-down / move focus left)
            (
                KeyCode::Backspace.into(),
                BoundCommand {
                    action: KeyboardAction::Back,
                    contexts: vec![KeyContext::Normal],
                    description: "Go back",
                },
            ),
            // Delete — Del / D (uppercase, with confirmation)
            (
                KeyCode::Delete.into(),
                BoundCommand {
                    action: KeyboardAction::Delete,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Delete item",
                },
            ),
            (
                KeyCode::Char('D').into(),
                BoundCommand {
                    action: KeyboardAction::Delete,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                    description: "Delete item",
                },
            ),
            // Direct tab switching by number — 1 through 2
            (
                KeyCode::Char('1').into(),
                BoundCommand {
                    action: KeyboardAction::SwitchTab(Tab::Library),
                    contexts: vec![KeyContext::Normal],
                    description: "Library tab",
                },
            ),
            (
                KeyCode::Char('2').into(),
                BoundCommand {
                    action: KeyboardAction::SwitchTab(Tab::Settings),
                    contexts: vec![KeyContext::Normal],
                    description: "Settings tab",
                },
            ),
            // Overlay triggers — Alt+key
            (
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::Queue),
                    contexts: vec![KeyContext::Normal],
                    description: "Queue overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::YTSearch),
                    contexts: vec![KeyContext::Normal],
                    description: "YouTube Search overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::SearchLibrary),
                    contexts: vec![KeyContext::Normal],
                    description: "Search Library overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::About),
                    contexts: vec![KeyContext::Normal],
                    description: "About overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::SleepTimer),
                    contexts: vec![KeyContext::Normal],
                    description: "Sleep Timer overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::ThemePicker),
                    contexts: vec![KeyContext::Normal],
                    description: "Theme Picker overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::Equalizer),
                    contexts: vec![KeyContext::Normal],
                    description: "Equalizer overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::CommandPalette),
                    contexts: vec![KeyContext::Normal],
                    description: "Command Palette overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::SpotifySearch),
                    contexts: vec![KeyContext::Normal],
                    description: "Spotify Search overlay",
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(OverlayId::SoundEffects),
                    contexts: vec![KeyContext::Normal],
                    description: "Sound Effects overlay",
                },
            ),
            // Footer preset cycling — Alt+F
            (
                KeyEvent::new(KeyCode::Char('F'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::CycleFooterPreset,
                    contexts: vec![KeyContext::Normal],
                    description: "Cycle footer preset",
                },
            ),
            // Library motions — vim-style
            // v — toggle multiselect mode
            (
                KeyCode::Char('v').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMultiselect,
                    contexts: vec![KeyContext::Normal],
                    description: "Toggle multiselect mode",
                },
            ),
            // a — add selected to queue
            (
                KeyCode::Char('a').into(),
                BoundCommand {
                    action: KeyboardAction::AddToQueue,
                    contexts: vec![KeyContext::Normal],
                    description: "Add to queue",
                },
            ),
            // A (Shift) — add to playlist
            (
                KeyCode::Char('A').into(),
                BoundCommand {
                    action: KeyboardAction::AddToPlaylist,
                    contexts: vec![KeyContext::Normal],
                    description: "Add to playlist",
                },
            ),
            // x — delete from list
            (
                KeyCode::Char('x').into(),
                BoundCommand {
                    action: KeyboardAction::DeleteFromList,
                    contexts: vec![KeyContext::Normal],
                    description: "Delete from list",
                },
            ),
            // gg — jump to start (handled via pending_motion in app.rs)
            // g alone is not bound; the app checks for double-press.
            // G (Shift) — jump to end
            (
                KeyCode::Char('G').into(),
                BoundCommand {
                    action: KeyboardAction::JumpToEnd,
                    contexts: vec![KeyContext::Normal],
                    description: "Jump to end",
                },
            ),
            // e — edit metadata
            (
                KeyCode::Char('e').into(),
                BoundCommand {
                    action: KeyboardAction::EditMetadata,
                    contexts: vec![KeyContext::Normal],
                    description: "Edit metadata",
                },
            ),
            // P — cycle progress style
            (
                KeyCode::Char('P').into(),
                BoundCommand {
                    action: KeyboardAction::CycleProgressStyle,
                    contexts: vec![KeyContext::Normal],
                    description: "Cycle progress style",
                },
            ),
            // Ctrl+V — toggle visualizer
            (
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::ToggleVisualizer,
                    contexts: vec![KeyContext::Normal],
                    description: "Toggle visualizer",
                },
            ),
        ],
    }
}
