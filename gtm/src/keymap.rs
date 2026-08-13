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
//!    Picker — modal picker is open
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::picker::PickerId;
use gtm_core::state::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    Global,
    Normal,
    List,
}

#[derive(Debug, Clone)]
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
    Delete,

    // Filter
    EnterFilter,

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
    OpenOverlay(PickerId),

    // Navigation
    Back,
    FocusLeft,
    FocusRight,

    // Lyrics
    FetchLyrics,

    // Library motions (vim-style)
    ToggleMultiselect,
    AddToQueue,
    AddToPlaylist,
    DeleteFromList,
    JumpToEnd,
    EditMetadata,

    // Meta
    Quit,
    QuitDaemon,
    ToggleHelp,
    HideHelpBar,
    CycleFooterPreset,
    CycleProgressStyle,
    CycleDesign,
    ToggleVisualizer,
    CheckHealth,
}

#[derive(Debug, Clone)]
pub struct BoundCommand {
    pub action: KeyboardAction,
    pub contexts: Vec<KeyContext>,
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
                },
            ),
            // Global — ToggleHelp
            (
                KeyCode::Char('?').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleHelp,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            // Ctrl+H — hide/show help bar in library view
            (
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::HideHelpBar,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Command palette (:)
            (
                KeyCode::Char(':').into(),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::CommandPalette),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Tab switching — Tab / Shift-Tab
            (
                KeyCode::Tab.into(),
                BoundCommand {
                    action: KeyboardAction::NextTab,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::BackTab.into(),
                BoundCommand {
                    action: KeyboardAction::PrevTab,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Numbered tab switching — 1 = Library, 2 = Settings
            (
                KeyCode::Char('1').into(),
                BoundCommand {
                    action: KeyboardAction::SwitchTab(Tab::Library),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('2').into(),
                BoundCommand {
                    action: KeyboardAction::SwitchTab(Tab::Settings),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Cursor — arrow keys
            (
                KeyCode::Up.into(),
                BoundCommand {
                    action: KeyboardAction::MoveUp,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Down.into(),
                BoundCommand {
                    action: KeyboardAction::MoveDown,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Cursor — vim-style j/k
            (
                KeyCode::Char('k').into(),
                BoundCommand {
                    action: KeyboardAction::MoveUp,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('j').into(),
                BoundCommand {
                    action: KeyboardAction::MoveDown,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Cursor — page up/down (PgUp/PgDn, Ctrl+U/Ctrl+D)
            (
                KeyCode::PageUp.into(),
                BoundCommand {
                    action: KeyboardAction::PageUp,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyCode::PageDown.into(),
                BoundCommand {
                    action: KeyboardAction::PageDown,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::PageUp,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::PageDown,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Cursor — jump to top/bottom (Home/End)
            (
                KeyCode::Home.into(),
                BoundCommand {
                    action: KeyboardAction::Top,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyCode::End.into(),
                BoundCommand {
                    action: KeyboardAction::Bottom,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Playback — space, n, p, Ctrl+N/P, s (stop)
            (
                KeyCode::Char(' ').into(),
                BoundCommand {
                    action: KeyboardAction::PlayPause,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('n').into(),
                BoundCommand {
                    action: KeyboardAction::Next,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('p').into(),
                BoundCommand {
                    action: KeyboardAction::Prev,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::Next,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::Prev,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('s').into(),
                BoundCommand {
                    action: KeyboardAction::Stop,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Volume — +, =, -
            (
                KeyCode::Char('+').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeUp,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('=').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeUp,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('-').into(),
                BoundCommand {
                    action: KeyboardAction::VolumeDown,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Mute — m
            (
                KeyCode::Char('m').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMute,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Quit daemon — Q / Ctrl+Q
            (
                KeyCode::Char('Q').into(),
                BoundCommand {
                    action: KeyboardAction::QuitDaemon,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::QuitDaemon,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            // Favourite — F
            (
                KeyCode::Char('F').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleFavourite,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Clear queue — D
            (
                KeyCode::Char('D').into(),
                BoundCommand {
                    action: KeyboardAction::ClearQueue,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Focus navigation — [, ]
            (
                KeyCode::Char('[').into(),
                BoundCommand {
                    action: KeyboardAction::FocusLeft,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char(']').into(),
                BoundCommand {
                    action: KeyboardAction::FocusRight,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Lyrics — l
            (
                KeyCode::Char('l').into(),
                BoundCommand {
                    action: KeyboardAction::FetchLyrics,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Repeat — r, Shift+R
            (
                KeyCode::Char('r').into(),
                BoundCommand {
                    action: KeyboardAction::CycleRepeat,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('R').into(),
                BoundCommand {
                    action: KeyboardAction::CycleRepeat,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Shuffle — Shift+S only (s is reserved for Stop)
            (
                KeyCode::Char('S').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleShuffle,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Seek — comma/period
            (
                KeyCode::Char('.').into(),
                BoundCommand {
                    action: KeyboardAction::SeekForward,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char(',').into(),
                BoundCommand {
                    action: KeyboardAction::SeekBackward,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Filter mode — /, Ctrl+F
            (
                KeyCode::Char('/').into(),
                BoundCommand {
                    action: KeyboardAction::EnterFilter,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::EnterFilter,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Select — Enter
            (
                KeyCode::Enter.into(),
                BoundCommand {
                    action: KeyboardAction::Select,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Back — Backspace (go back in library drill-down / move focus left)
            (
                KeyCode::Backspace.into(),
                BoundCommand {
                    action: KeyboardAction::Back,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Delete — Del / d (with confirmation)
            (
                KeyCode::Delete.into(),
                BoundCommand {
                    action: KeyboardAction::Delete,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            (
                KeyCode::Char('d').into(),
                BoundCommand {
                    action: KeyboardAction::Delete,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Overlay triggers — Alt+key
            (
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::Queue),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::YTSearch),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::SearchLibrary),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::About),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::SleepTimer),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::ThemePicker),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::Equalizer),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::CommandPalette),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::SpotifySearch),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::SoundEffects),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Footer preset cycling — Alt+F
            (
                KeyEvent::new(KeyCode::Char('F'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::CycleFooterPreset,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Design cycling — Alt+D
            (
                KeyEvent::new(KeyCode::Char('D'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::CycleDesign,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Library motions — vim-style
            // v — toggle multiselect mode
            (
                KeyCode::Char('v').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMultiselect,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // a — add selected to queue
            (
                KeyCode::Char('a').into(),
                BoundCommand {
                    action: KeyboardAction::AddToQueue,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // A (Shift) — add to playlist
            (
                KeyCode::Char('A').into(),
                BoundCommand {
                    action: KeyboardAction::AddToPlaylist,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // x — delete from list
            (
                KeyCode::Char('x').into(),
                BoundCommand {
                    action: KeyboardAction::DeleteFromList,
                    contexts: vec![KeyContext::Normal],
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
                },
            ),
            // e — edit metadata
            (
                KeyCode::Char('e').into(),
                BoundCommand {
                    action: KeyboardAction::EditMetadata,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // P — cycle progress style
            (
                KeyCode::Char('P').into(),
                BoundCommand {
                    action: KeyboardAction::CycleProgressStyle,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Ctrl+V — toggle visualizer
            (
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::ToggleVisualizer,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // : (colon) — health check (neovim-style)
            (
                KeyCode::Char(':').into(),
                BoundCommand {
                    action: KeyboardAction::CheckHealth,
                    contexts: vec![KeyContext::Normal],
                },
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(key: KeyEvent, ctx: KeyContext) -> Option<KeyboardAction> {
        default_keybindings().dispatch(key, ctx)
    }

    #[test]
    fn delete_uses_lowercase_d_and_del() {
        // d / Del remove an item…
        assert!(matches!(
            dispatch(KeyCode::Char('d').into(), KeyContext::Normal),
            Some(KeyboardAction::Delete)
        ));
        assert!(matches!(
            dispatch(KeyCode::Delete.into(), KeyContext::Normal),
            Some(KeyboardAction::Delete)
        ));
        // …while uppercase D still clears the queue.
        assert!(matches!(
            dispatch(KeyCode::Char('D').into(), KeyContext::Normal),
            Some(KeyboardAction::ClearQueue)
        ));
    }

    #[test]
    fn numbered_tabs_switch_views() {
        assert!(matches!(
            dispatch(KeyCode::Char('1').into(), KeyContext::Normal),
            Some(KeyboardAction::SwitchTab(Tab::Library))
        ));
        assert!(matches!(
            dispatch(KeyCode::Char('2').into(), KeyContext::Normal),
            Some(KeyboardAction::SwitchTab(Tab::Settings))
        ));
    }

    #[test]
    fn quit_keyboard_actions() {
        assert!(matches!(
            dispatch(KeyCode::Char('q').into(), KeyContext::Global),
            Some(KeyboardAction::Quit)
        ));
        assert!(matches!(
            dispatch(KeyCode::Char('Q').into(), KeyContext::Global),
            Some(KeyboardAction::QuitDaemon)
        ));
    }
}
