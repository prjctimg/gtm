// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Keybinding dispatch with context-aware matching
//
// This is free software released under the GPL-3.0 license.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::picker::PickerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    Global,
    Normal,
    List,
}

#[derive(Debug, Clone)]
pub enum KeyboardAction {
    // Pane cycling (Tab / Shift-Tab)
    NextPane,
    PrevPane,

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
    ToggleVisualizer,
    ToggleTheme,
    CycleSort,
    CheckHealth,

    // Queue move mode
    QueueMoveUp,
    QueueMoveDown,
    QueueMoveConfirm,
    QueueMoveCancel,
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
    if event.code != binding.code {
        return false;
    }
    // Chord modifiers (Ctrl/Alt/Cmd) must match exactly when a binding
    // specifies them, and a binding recorded without chords never fires for
    // a chord-carrying press: some terminals drop CONTROL on punctuation,
    // which used to let plain playback bindings (e.g. `,` seek) swallow
    // shortcuts like Ctrl+, (Settings picker).
    let chords = KeyModifiers::CONTROL
        | KeyModifiers::ALT
        | KeyModifiers::SUPER
        | KeyModifiers::META
        | KeyModifiers::HYPER;
    if event.modifiers.intersects(chords) || binding.modifiers.intersects(chords) {
        return event.modifiers == binding.modifiers;
    }
    // Shift alone is fuzzy: terminals report it inconsistently for letters.
    true
}

/// Build the default set of key bindings.  Layered by context:
///
///   Global : q (quit), ? (help), space (play/pause)
///   Normal : tab switching, cursor, volume, filters, playback control
///   List   : j/k, enter, delete
///
/// Bindings are scanned in order; the first match wins.
pub fn default_keybindings() -> Keybindings {
    Keybindings {
        bindings: vec![
            // Global: Quit
            (
                KeyCode::Char('q').into(),
                BoundCommand {
                    action: KeyboardAction::Quit,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            // Global: ToggleHelp
            (
                KeyCode::Char('?').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleHelp,
                    contexts: vec![KeyContext::Global, KeyContext::Normal],
                },
            ),
            // Ctrl+H: hide/show help bar in library view
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
            // Pane cycling: Tab / Shift-Tab
            (
                KeyCode::Tab.into(),
                BoundCommand {
                    action: KeyboardAction::NextPane,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyCode::BackTab.into(),
                BoundCommand {
                    action: KeyboardAction::PrevPane,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Alt+,: open settings picker
            (
                KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::Settings),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Cursor: arrow keys
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
            // Cursor: vim-style j/k
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
            // Cursor: page up/down (PgUp/PgDn, Ctrl+U/Ctrl+D)
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
            // Cursor: jump to top/bottom (Home/End)
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
            // Playback: space, n, p, Ctrl+N/P, s (stop)
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
                KeyCode::Char('s').into(),
                BoundCommand {
                    action: KeyboardAction::Stop,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Volume: +, =, -
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
            // Mute: m
            (
                KeyCode::Char('m').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMute,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Quit daemon: Q / Ctrl+Q
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
            // Favourite: f
            (
                KeyCode::Char('f').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleFavourite,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Clear queue: D
            (
                KeyCode::Char('D').into(),
                BoundCommand {
                    action: KeyboardAction::ClearQueue,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Focus navigation: [, ]
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
            // Lyrics: l
            (
                KeyCode::Char('l').into(),
                BoundCommand {
                    action: KeyboardAction::FetchLyrics,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Repeat: r, Shift+R
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
            // Shuffle: Shift+S only (s is reserved for Stop)
            (
                KeyCode::Char('S').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleShuffle,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Seek: comma/period
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
            // Filter mode: Ctrl+F (the '/' key now opens the library search picker)
            (
                KeyCode::Char('/').into(),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::SearchLibrary),
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
            // Select: Enter
            (
                KeyCode::Enter.into(),
                BoundCommand {
                    action: KeyboardAction::Select,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Back: Backspace (go back in library drill-down / move focus left)
            (
                KeyCode::Backspace.into(),
                BoundCommand {
                    action: KeyboardAction::Back,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Delete: Del / d (with confirmation)
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
            // Overlay triggers: Alt+key
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
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::ALT),
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
                    action: KeyboardAction::OpenOverlay(PickerId::ProgressStyle),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::VisualizerPreset),
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
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::OpenOverlay(PickerId::Notifications),
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Library motions: vim-style
            // v: toggle multiselect mode
            (
                KeyCode::Char('v').into(),
                BoundCommand {
                    action: KeyboardAction::ToggleMultiselect,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // a: add selected to queue
            (
                KeyCode::Char('a').into(),
                BoundCommand {
                    action: KeyboardAction::AddToQueue,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // A (Shift): add to playlist
            (
                KeyCode::Char('A').into(),
                BoundCommand {
                    action: KeyboardAction::AddToPlaylist,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // x: delete from list
            (
                KeyCode::Char('x').into(),
                BoundCommand {
                    action: KeyboardAction::DeleteFromList,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // gg: jump to start (handled via pending_motion in app.rs)
            // g alone is not bound; the app checks for double-press.
            // G (Shift): jump to end
            (
                KeyCode::Char('G').into(),
                BoundCommand {
                    action: KeyboardAction::JumpToEnd,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // e: edit metadata
            (
                KeyCode::Char('e').into(),
                BoundCommand {
                    action: KeyboardAction::EditMetadata,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Ctrl+V: toggle visualizer
            (
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::ToggleVisualizer,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Alt+T: cycle theme (dark/light variants)
            (
                KeyEvent::new(KeyCode::Char('T'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::ToggleTheme,
                    contexts: vec![KeyContext::Normal],
                },
            ),
            // Alt+S: cycle the library track list sort order
            (
                KeyEvent::new(KeyCode::Char('S'), KeyModifiers::ALT),
                BoundCommand {
                    action: KeyboardAction::CycleSort,
                    contexts: vec![KeyContext::List, KeyContext::Normal],
                },
            ),
            // Queue move mode: Ctrl+j/k to move, Enter to confirm, Esc to cancel
            (
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::QueueMoveDown,
                    contexts: vec![KeyContext::List],
                },
            ),
            (
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                BoundCommand {
                    action: KeyboardAction::QueueMoveUp,
                    contexts: vec![KeyContext::List],
                },
            ),
            (
                KeyCode::Enter.into(),
                BoundCommand {
                    action: KeyboardAction::QueueMoveConfirm,
                    contexts: vec![KeyContext::List],
                },
            ),
            (
                KeyCode::Esc.into(),
                BoundCommand {
                    action: KeyboardAction::QueueMoveCancel,
                    contexts: vec![KeyContext::List],
                },
            ),
        ],
    }
}

/// Parse a key string like `"Ctrl+q"`, `"Space"`, `"Alt+s"`, `"Enter"` into a
/// `KeyEvent`.  Returns `None` on unrecognised tokens.
pub fn parse_key_event(s: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::NONE;
    let mut code = None;
    for part in s.split('+') {
        let p = part.trim();
        match p.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "opt" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "super" | "meta" | "cmd" | "command" => modifiers |= KeyModifiers::SUPER,
            _ => {
                let c = match p {
                    "space" => KeyCode::Char(' '),
                    "enter" | "return" => KeyCode::Enter,
                    "tab" => KeyCode::Tab,
                    "backtab" | "back-tab" | "shift+tab" => KeyCode::BackTab,
                    "backspace" | "bs" => KeyCode::Backspace,
                    "delete" | "del" => KeyCode::Delete,
                    "esc" | "escape" => KeyCode::Esc,
                    "home" => KeyCode::Home,
                    "end" => KeyCode::End,
                    "pageup" | "page_up" | "pgup" => KeyCode::PageUp,
                    "pagedown" | "page_down" | "pgdn" => KeyCode::PageDown,
                    "up" => KeyCode::Up,
                    "down" => KeyCode::Down,
                    "left" => KeyCode::Left,
                    "right" => KeyCode::Right,
                    "ins" | "insert" => KeyCode::Insert,
                    "f1" => KeyCode::F(1),
                    "f2" => KeyCode::F(2),
                    "f3" => KeyCode::F(3),
                    "f4" => KeyCode::F(4),
                    "f5" => KeyCode::F(5),
                    "f6" => KeyCode::F(6),
                    "f7" => KeyCode::F(7),
                    "f8" => KeyCode::F(8),
                    "f9" => KeyCode::F(9),
                    "f10" => KeyCode::F(10),
                    "f11" => KeyCode::F(11),
                    "f12" => KeyCode::F(12),
                    _ if p.len() == 1 => KeyCode::Char(p.chars().next()?),
                    _ => return None,
                };
                code = Some(c);
            }
        }
    }
    Some(KeyEvent::new(code?, modifiers))
}

impl KeyboardAction {
    /// Map an action name (from config) to a `KeyboardAction`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "next_pane" => KeyboardAction::NextPane,
            "prev_pane" => KeyboardAction::PrevPane,
            "move_up" | "up" => KeyboardAction::MoveUp,
            "move_down" | "down" => KeyboardAction::MoveDown,
            "page_up" => KeyboardAction::PageUp,
            "page_down" => KeyboardAction::PageDown,
            "top" | "home" => KeyboardAction::Top,
            "bottom" | "end" => KeyboardAction::Bottom,
            "select" | "enter" => KeyboardAction::Select,
            "delete" | "del" => KeyboardAction::Delete,
            "enter_filter" | "filter" => KeyboardAction::EnterFilter,
            "play_pause" | "toggle_playback" => KeyboardAction::PlayPause,
            "next" => KeyboardAction::Next,
            "prev" | "previous" => KeyboardAction::Prev,
            "stop" => KeyboardAction::Stop,
            "volume_up" | "vol_up" => KeyboardAction::VolumeUp,
            "volume_down" | "vol_down" => KeyboardAction::VolumeDown,
            "seek_forward" | "seek_fwd" => KeyboardAction::SeekForward,
            "seek_backward" | "seek_back" => KeyboardAction::SeekBackward,
            "toggle_shuffle" | "shuffle" => KeyboardAction::ToggleShuffle,
            "cycle_repeat" | "repeat" => KeyboardAction::CycleRepeat,
            "toggle_mute" | "mute" => KeyboardAction::ToggleMute,
            "toggle_favourite" | "favourite" | "fav" => KeyboardAction::ToggleFavourite,
            "clear_queue" => KeyboardAction::ClearQueue,
            "back" => KeyboardAction::Back,
            "focus_left" => KeyboardAction::FocusLeft,
            "focus_right" => KeyboardAction::FocusRight,
            "fetch_lyrics" | "lyrics" => KeyboardAction::FetchLyrics,
            "toggle_multiselect" | "multiselect" => KeyboardAction::ToggleMultiselect,
            "add_to_queue" | "enqueue" => KeyboardAction::AddToQueue,
            "add_to_playlist" => KeyboardAction::AddToPlaylist,
            "delete_from_list" => KeyboardAction::DeleteFromList,
            "jump_to_end" | "jump_end" | "G" => KeyboardAction::JumpToEnd,
            "edit_metadata" | "edit" => KeyboardAction::EditMetadata,
            "quit" => KeyboardAction::Quit,
            "quit_daemon" | "quit_all" => KeyboardAction::QuitDaemon,
            "toggle_help" | "help" => KeyboardAction::ToggleHelp,
            "hide_help_bar" => KeyboardAction::HideHelpBar,
            "toggle_visualizer" | "visualizer" | "vis" => KeyboardAction::ToggleVisualizer,
            "toggle_theme" | "theme" => KeyboardAction::ToggleTheme,
            "cycle_sort" | "sort" => KeyboardAction::CycleSort,
            "check_health" | "health" => KeyboardAction::CheckHealth,
            "queue_move_up" => KeyboardAction::QueueMoveUp,
            "queue_move_down" => KeyboardAction::QueueMoveDown,
            "queue_move_confirm" => KeyboardAction::QueueMoveConfirm,
            "queue_move_cancel" => KeyboardAction::QueueMoveCancel,
            // Overlay openers
            "open_queue" => KeyboardAction::OpenOverlay(PickerId::Queue),
            "open_yt_search" | "open_youtube" => KeyboardAction::OpenOverlay(PickerId::YTSearch),
            "open_search" | "open_library_search" => {
                KeyboardAction::OpenOverlay(PickerId::SearchLibrary)
            }
            "open_settings" | "settings" => KeyboardAction::OpenOverlay(PickerId::Settings),
            "open_spotify_search" | "open_spotify" => {
                KeyboardAction::OpenOverlay(PickerId::SpotifySearch)
            }
            "open_notifications" | "notifications" => {
                KeyboardAction::OpenOverlay(PickerId::Notifications)
            }
            "open_theme_picker" | "themes" => KeyboardAction::OpenOverlay(PickerId::ThemePicker),
            "open_eq" | "open_equalizer" | "equalizer" => {
                KeyboardAction::OpenOverlay(PickerId::Equalizer)
            }
            "open_progress_style" => KeyboardAction::OpenOverlay(PickerId::ProgressStyle),
            "open_visualizer_preset" => KeyboardAction::OpenOverlay(PickerId::VisualizerPreset),
            "open_about" | "about" => KeyboardAction::OpenOverlay(PickerId::About),
            "open_sleep_timer" | "sleep_timer" => KeyboardAction::OpenOverlay(PickerId::SleepTimer),
            "open_command_palette" | "commands" => {
                KeyboardAction::OpenOverlay(PickerId::CommandPalette)
            }
            _ => return None,
        })
    }
}

/// Detect clashes in a set of user-defined bindings.
///
/// Returns a list of warning strings for each pair of bindings that share the
/// same `KeyEvent` and have at least one overlapping `KeyContext`.
pub fn detect_clashes(bindings: &[(KeyEvent, String, Vec<KeyContext>)]) -> Vec<String> {
    let mut warnings = Vec::new();
    for (i, (ki, action_i, ctx_i)) in bindings.iter().enumerate() {
        for (kj, action_j, ctx_j) in &bindings[i + 1..] {
            if key_matches(ki, kj) {
                let overlap: Vec<_> = ctx_i.iter().filter(|c| ctx_j.contains(c)).collect();
                if !overlap.is_empty() {
                    let ctx_names: Vec<_> = overlap.iter().map(|c| format!("{:?}", c)).collect();
                    warnings.push(format!(
                        "\"{}\" and \"{}\" share key {:?} in [{}]",
                        action_i,
                        action_j,
                        ki.code,
                        ctx_names.join(", ")
                    ));
                }
            }
        }
    }
    warnings
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
    fn settings_picker_shortcut_not_shadowed_by_chords() {
        // Alt+, opens the Settings picker; plain `,` seeks backward.
        assert!(matches!(
            dispatch(
                KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT),
                KeyContext::Normal
            ),
            Some(KeyboardAction::OpenOverlay(PickerId::Settings))
        ));
        assert!(matches!(
            dispatch(KeyCode::Char(',').into(), KeyContext::Normal),
            Some(KeyboardAction::SeekBackward)
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

    #[test]
    fn colon_opens_command_palette_only() {
        // `:` is the command palette; the removed duplicate (health check)
        // binding must not shadow it.
        assert!(matches!(
            dispatch(KeyCode::Char(':').into(), KeyContext::Normal),
            Some(KeyboardAction::OpenOverlay(PickerId::CommandPalette))
        ));
    }
}
