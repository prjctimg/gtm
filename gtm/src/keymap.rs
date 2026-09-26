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
    /// Contextual search: picks the search backend matching the focused list.
    Search,

    // Playback
    PlayPause,
    Next,
    Prev,
    Stop,
    VolumeUp,
    VolumeDown,
    SpeedUp,
    SpeedDown,
    ToggleLowPower,
    /// Fullscreen "Zen mode" showing one of the enlarged-cover + progress,
    /// lyrics, or visualizer surfaces at a time (`z`).
    ToggleZen,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    CycleRepeat,
    ToggleMute,
    /// Toggle mono downmix (`Alt+1`).
    ToggleMono,
    ToggleFavourite,
    /// Love / un-love the current track on Last.fm (`*`).
    ToggleLove,
    /// Save the track on air to Spotify Liked Songs (`L`).
    LikeSpotify,
    /// Pick Spotify destinations (Liked Songs and/or playlists) for the track
    /// on air (`Alt+L`).
    AddToSpotify,
    /// Toggle Last.fm scrobbling for this session (`&`).
    ToggleScrobble,

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

    // Multiselect
    MultiselectUp,
    MultiselectDown,
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
/// Build the default set of key bindings.  Layered by context:
///
///   Global : q (quit), ? (help), space (play/pause)
///   Normal : tab switching, cursor, volume, filters, playback control
///   List   : j/k, enter, delete
///
/// Bindings are scanned in order; the first match wins.
///
/// The table is a flat list of `(key, action, contexts)` triples and `b!`
/// expands one triple into the `(KeyEvent, BoundCommand)` pair, so adding a
/// binding is a single line instead of a seven-line struct literal.
macro_rules! b {
    ($key:expr, $action:expr, GLOBAL) => {
        (
            $key.into(),
            BoundCommand {
                action: $action,
                contexts: vec![KeyContext::Global, KeyContext::Normal],
            },
        )
    };
    ($key:expr, $action:expr, NORMAL) => {
        (
            $key.into(),
            BoundCommand {
                action: $action,
                contexts: vec![KeyContext::Normal],
            },
        )
    };
    ($key:expr, $action:expr, LIST) => {
        (
            $key.into(),
            BoundCommand {
                action: $action,
                contexts: vec![KeyContext::List, KeyContext::Normal],
            },
        )
    };
    ($key:expr, $action:expr, LIST_ONLY) => {
        (
            $key.into(),
            BoundCommand {
                action: $action,
                contexts: vec![KeyContext::List],
            },
        )
    };
}

pub fn default_keybindings() -> Keybindings {
    Keybindings {
        bindings: vec![
            b!(KeyCode::Char('q'), KeyboardAction::Quit, GLOBAL),
            b!(KeyCode::Char('?'), KeyboardAction::ToggleHelp, GLOBAL),
            b!(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
                KeyboardAction::HideHelpBar,
                NORMAL
            ),
            b!(
                KeyCode::Char(':'),
                KeyboardAction::OpenOverlay(PickerId::CommandPalette),
                NORMAL
            ),
            b!(KeyCode::Tab, KeyboardAction::NextPane, NORMAL),
            b!(KeyCode::BackTab, KeyboardAction::PrevPane, NORMAL),
            b!(
                KeyEvent::new(KeyCode::Char(','), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Settings),
                NORMAL
            ),
            b!(KeyCode::Up, KeyboardAction::MoveUp, LIST),
            b!(KeyCode::Down, KeyboardAction::MoveDown, LIST),
            b!(
                KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
                KeyboardAction::MultiselectUp,
                LIST
            ),
            b!(
                KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
                KeyboardAction::MultiselectDown,
                LIST
            ),
            b!(KeyCode::Char('k'), KeyboardAction::MoveUp, LIST),
            b!(KeyCode::Char('j'), KeyboardAction::MoveDown, LIST),
            b!(KeyCode::PageUp, KeyboardAction::PageUp, LIST),
            b!(KeyCode::PageDown, KeyboardAction::PageDown, LIST),
            b!(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                KeyboardAction::PageUp,
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
                KeyboardAction::PageDown,
                NORMAL
            ),
            b!(KeyCode::Home, KeyboardAction::Top, LIST),
            b!(KeyCode::End, KeyboardAction::Bottom, LIST),
            b!(KeyCode::Char(' '), KeyboardAction::PlayPause, GLOBAL),
            b!(KeyCode::Char('n'), KeyboardAction::Next, GLOBAL),
            b!(KeyCode::Char('p'), KeyboardAction::Prev, GLOBAL),
            b!(KeyCode::Char('s'), KeyboardAction::Stop, NORMAL),
            b!(KeyCode::Char('+'), KeyboardAction::VolumeUp, NORMAL),
            b!(KeyCode::Char('='), KeyboardAction::VolumeUp, NORMAL),
            b!(KeyCode::Char('-'), KeyboardAction::VolumeDown, NORMAL),
            b!(KeyCode::Char('>'), KeyboardAction::SpeedUp, NORMAL),
            b!(KeyCode::Char('<'), KeyboardAction::SpeedDown, NORMAL),
            b!(KeyCode::Char('z'), KeyboardAction::ToggleZen, NORMAL),
            b!(KeyCode::Char('m'), KeyboardAction::ToggleMute, NORMAL),
            b!(
                KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
                KeyboardAction::ToggleMono,
                NORMAL
            ),
            b!(KeyCode::Char('Q'), KeyboardAction::QuitDaemon, GLOBAL),
            b!(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
                KeyboardAction::QuitDaemon,
                GLOBAL
            ),
            b!(KeyCode::Char('f'), KeyboardAction::ToggleFavourite, NORMAL),
            b!(KeyCode::Char('*'), KeyboardAction::ToggleLove, NORMAL),
            b!(KeyCode::Char('L'), KeyboardAction::LikeSpotify, NORMAL),
            b!(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT),
                KeyboardAction::AddToSpotify,
                NORMAL
            ),
            b!(KeyCode::Char('&'), KeyboardAction::ToggleScrobble, NORMAL),
            b!(KeyCode::Char('D'), KeyboardAction::ClearQueue, NORMAL),
            b!(KeyCode::Char('['), KeyboardAction::FocusLeft, NORMAL),
            b!(KeyCode::Char(']'), KeyboardAction::FocusRight, NORMAL),
            b!(KeyCode::Char('l'), KeyboardAction::FetchLyrics, NORMAL),
            b!(KeyCode::Char('r'), KeyboardAction::CycleRepeat, NORMAL),
            b!(KeyCode::Char('R'), KeyboardAction::CycleRepeat, NORMAL),
            b!(KeyCode::Char('S'), KeyboardAction::ToggleShuffle, NORMAL),
            b!(KeyCode::Char('.'), KeyboardAction::SeekForward, NORMAL),
            b!(KeyCode::Char(','), KeyboardAction::SeekBackward, NORMAL),
            b!(KeyCode::Char('/'), KeyboardAction::Search, NORMAL),
            b!(
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
                KeyboardAction::EnterFilter,
                NORMAL
            ),
            b!(KeyCode::Enter, KeyboardAction::Select, LIST),
            b!(KeyCode::Backspace, KeyboardAction::Back, NORMAL),
            b!(KeyCode::Delete, KeyboardAction::Delete, LIST),
            b!(KeyCode::Char('d'), KeyboardAction::Delete, LIST),
            b!(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Queue),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::YTSearch),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::ALT),
                KeyboardAction::Search,
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::About),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::SleepTimer),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::ThemePicker),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Equalizer),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::ProgressStyle),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::VisualizerPreset),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::SpotifySearch),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::LoadStream),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::PodcastFeeds),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Radio),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Notifications),
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
                KeyboardAction::OpenOverlay(PickerId::Setup),
                NORMAL
            ),
            b!(
                KeyCode::Char('v'),
                KeyboardAction::ToggleMultiselect,
                NORMAL
            ),
            b!(KeyCode::Char('a'), KeyboardAction::AddToQueue, NORMAL),
            b!(KeyCode::Char('A'), KeyboardAction::AddToPlaylist, NORMAL),
            b!(KeyCode::Char('x'), KeyboardAction::DeleteFromList, NORMAL),
            b!(KeyCode::Char('G'), KeyboardAction::JumpToEnd, NORMAL),
            b!(KeyCode::Char('e'), KeyboardAction::EditMetadata, NORMAL),
            b!(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                KeyboardAction::ToggleVisualizer,
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('T'), KeyModifiers::ALT),
                KeyboardAction::ToggleTheme,
                NORMAL
            ),
            b!(
                KeyEvent::new(KeyCode::Char('S'), KeyModifiers::ALT),
                KeyboardAction::CycleSort,
                LIST
            ),
            b!(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                KeyboardAction::QueueMoveDown,
                LIST_ONLY
            ),
            b!(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                KeyboardAction::QueueMoveUp,
                LIST_ONLY
            ),
            b!(KeyCode::Enter, KeyboardAction::QueueMoveConfirm, LIST_ONLY),
            b!(KeyCode::Esc, KeyboardAction::QueueMoveCancel, LIST_ONLY),
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
                // Named keys match case-insensitively, like the modifier arms
                // above, so `"Space"` and `"Ctrl+Space"` both work. The
                // single-character fallback keeps the original casing: `S` and
                // `s` are different bindings.
                let c = match p.to_lowercase().as_str() {
                    "space" => KeyCode::Char(' '),
                    "enter" | "return" => KeyCode::Enter,
                    "tab" => {
                        if modifiers.contains(KeyModifiers::SHIFT) {
                            KeyCode::BackTab
                        } else {
                            KeyCode::Tab
                        }
                    }
                    "backtab" | "back-tab" => KeyCode::BackTab,
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
                    // `chars().count()`, not `len()`: a single non-ASCII
                    // character is more than one byte.
                    _ if p.chars().count() == 1 => KeyCode::Char(p.chars().next()?),
                    _ => return None,
                };
                code = Some(c);
            }
        }
    }
    Some(KeyEvent::new(code?, modifiers))
}

/// Inverse of [`parse_key_event`]: the spelling of a key event as it appears in
/// `config.toml` and the help screen. Lets the footer echo the key a command
/// was triggered with instead of the command's name.
///
/// A shifted letter arrives as an already-uppercase `Char` with the `SHIFT`
/// modifier set, so the modifier is folded into the character rather than
/// spelled out; every other shifted key keeps an explicit `Shift+`.
pub fn format_key_event(key: &KeyEvent) -> String {
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("Alt+");
    }
    if key.modifiers.contains(KeyModifiers::SUPER) {
        out.push_str("Super+");
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if shift && !matches!(key.code, KeyCode::Char(_)) {
        out.push_str("Shift+");
    }
    match key.code {
        KeyCode::Char(' ') => out.push_str("Space"),
        KeyCode::Char(c) => out.push(if shift { c.to_ascii_uppercase() } else { c }),
        KeyCode::Enter => out.push_str("Enter"),
        KeyCode::Tab => out.push_str("Tab"),
        KeyCode::BackTab => out.push_str("BackTab"),
        KeyCode::Backspace => out.push_str("Backspace"),
        KeyCode::Delete => out.push_str("Delete"),
        KeyCode::Esc => out.push_str("Esc"),
        KeyCode::Home => out.push_str("Home"),
        KeyCode::End => out.push_str("End"),
        KeyCode::PageUp => out.push_str("PageUp"),
        KeyCode::PageDown => out.push_str("PageDown"),
        KeyCode::Up => out.push_str("Up"),
        KeyCode::Down => out.push_str("Down"),
        KeyCode::Left => out.push_str("Left"),
        KeyCode::Right => out.push_str("Right"),
        KeyCode::Insert => out.push_str("Insert"),
        KeyCode::F(n) => {
            out.push('F');
            out.push_str(&n.to_string());
        }
        // A key the user has not bound; showing nothing would be worse than
        // an honest placeholder.
        _ => out.push('?'),
    }
    out
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
            "search" => KeyboardAction::Search,
            "play_pause" | "toggle_playback" => KeyboardAction::PlayPause,
            "next" => KeyboardAction::Next,
            "prev" | "previous" => KeyboardAction::Prev,
            "stop" => KeyboardAction::Stop,
            "volume_up" | "vol_up" => KeyboardAction::VolumeUp,
            "volume_down" | "vol_down" => KeyboardAction::VolumeDown,
            "speed_up" => KeyboardAction::SpeedUp,
            "speed_down" => KeyboardAction::SpeedDown,
            "toggle_low_power" | "low_power" => KeyboardAction::ToggleLowPower,
            "toggle_zen" | "zen" => KeyboardAction::ToggleZen,
            "seek_forward" | "seek_fwd" => KeyboardAction::SeekForward,
            "seek_backward" | "seek_back" => KeyboardAction::SeekBackward,
            "toggle_shuffle" | "shuffle" => KeyboardAction::ToggleShuffle,
            "cycle_repeat" | "repeat" => KeyboardAction::CycleRepeat,
            "toggle_mute" | "mute" => KeyboardAction::ToggleMute,
            "toggle_mono" | "mono" => KeyboardAction::ToggleMono,
            "toggle_favourite" | "favourite" | "fav" => KeyboardAction::ToggleFavourite,
            "toggle_love" | "love" => KeyboardAction::ToggleLove,
            "like_spotify" | "spotify_like" => KeyboardAction::LikeSpotify,
            "add_to_spotify" | "spotify_add" => KeyboardAction::AddToSpotify,
            "toggle_scrobble" | "scrobble" => KeyboardAction::ToggleScrobble,
            "clear_queue" => KeyboardAction::ClearQueue,
            "back" => KeyboardAction::Back,
            "focus_left" => KeyboardAction::FocusLeft,
            "focus_right" => KeyboardAction::FocusRight,
            "fetch_lyrics" | "lyrics" => KeyboardAction::FetchLyrics,
            "toggle_multiselect" | "multiselect" => KeyboardAction::ToggleMultiselect,
            "multiselect_up" => KeyboardAction::MultiselectUp,
            "multiselect_down" => KeyboardAction::MultiselectDown,
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
            "open_search" | "open_library_search" => KeyboardAction::Search,
            "open_settings" | "settings" => KeyboardAction::OpenOverlay(PickerId::Settings),
            "open_spotify_search" | "open_spotify" => {
                KeyboardAction::OpenOverlay(PickerId::SpotifySearch)
            }
            "open_podcast" | "open_podcasts" => KeyboardAction::OpenOverlay(PickerId::PodcastFeeds),
            "open_radio" | "open_radios" | "open_radio_browse" | "browse_radio" => {
                KeyboardAction::OpenOverlay(PickerId::Radio)
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
            "open_setup" | "setup" => KeyboardAction::OpenOverlay(PickerId::Setup),
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
    fn delete_lowercase_keys() {
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
    fn picker_shortcut_unique() {
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
    fn colon_opens_palette() {
        // `:` is the command palette; the removed duplicate (health check)
        // binding must not shadow it.
        assert!(matches!(
            dispatch(KeyCode::Char(':').into(), KeyContext::Normal),
            Some(KeyboardAction::OpenOverlay(PickerId::CommandPalette))
        ));
    }

    #[test]
    fn parse_named_keys_ignore_case_but_chars_do_not() {
        // Modifier arms have always been case-insensitive, so `Ctrl+d` parsed
        // while `Space` did not. Named keys must behave the same way, and the
        // single-character fallback must keep its case or `S` and `s` would
        // collide.
        let space = KeyCode::Char(' ').into();
        assert_eq!(parse_key_event("Space"), Some(space));
        assert_eq!(parse_key_event("space"), Some(space));
        assert_eq!(parse_key_event("SPACE"), Some(space));
        assert_eq!(
            parse_key_event("Shift+Space"),
            Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT))
        );
        let upper = KeyCode::Char('S').into();
        assert_eq!(parse_key_event("S"), Some(upper));
        assert_ne!(parse_key_event("S"), parse_key_event("s"));
        // A single non-ASCII character is one key, not two bytes.
        assert_eq!(
            parse_key_event("é"),
            Some(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE))
        );
    }

    #[test]
    fn format_is_readable() {
        let show = |k: KeyEvent| format_key_event(&k);
        assert_eq!(show(KeyCode::Char('n').into()), "n");
        assert_eq!(show(KeyCode::Char(' ').into()), "Space");
        assert_eq!(show(KeyCode::Up.into()), "Up");
        assert_eq!(show(KeyCode::F(5).into()), "F5");
        assert_eq!(
            show(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            "Ctrl+d"
        );
        assert_eq!(
            show(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT)),
            "Alt+1"
        );
    }

    #[test]
    fn shifted_letter_has_no_shift_prefix() {
        // A terminal sends `S` with SHIFT set for shift+s; the character
        // already carries the shift, so `Shift+S` would be redundant.
        assert_eq!(
            format_key_event(&KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            "S"
        );
        // A shifted non-letter still needs the modifier spelled out.
        assert_eq!(
            format_key_event(&KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT)),
            "Shift+Up"
        );
    }

    #[test]
    fn format_round_trips_through_parse() {
        // `config.toml` spells bindings with `parse_key_event`; the footer
        // echo must be a string a user could paste back into `keybindings` and
        // still get the same binding. The key must survive, and the modifiers
        // must still *match* — not be spelled identically, since `key_matches`
        // deliberately treats a lone Shift as fuzzy.
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('S'), KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
        ] {
            let text = format_key_event(&key);
            let back =
                parse_key_event(&text).unwrap_or_else(|| panic!("{text} did not parse back"));
            assert_eq!(back.code, key.code, "{text} changed the key");
            assert!(
                key_matches(&key, &back),
                "{text} no longer matches the press it came from"
            );
        }
    }
}
