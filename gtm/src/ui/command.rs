// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Command palette entries
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

#[derive(Debug)]
pub struct Command {
    /// What the palette shows. The glyph that used to lead it is gone: eleven
    /// rows of icons in a list the user reads by name is a column of noise, and
    /// the icon style was the only reason this was two tables.
    pub label: &'static str,
    pub keys: &'static str,
    pub hint: &'static str,
}

pub struct CommandPalette;

impl CommandPalette {
    /// Every command the palette offers.
    ///
    /// One table. It was two — a nerd-font glyph and an emoji glyph per entry —
    /// and they had already drifted: each had one command the other lacked, so
    /// the palette showed "About" to emoji users and "Load Stream" to everyone
    /// else, from the same list.
    pub(crate) fn commands() -> &'static [Command] {
        &[
            Command {
                label: "Play/Pause",
                keys: "Space",
                hint: "play/pause",
            },
            Command {
                label: "Next Track",
                keys: "n",
                hint: "next track",
            },
            Command {
                label: "Prev Track",
                keys: "p",
                hint: "prev track",
            },
            Command {
                label: "Stop",
                keys: "s",
                hint: "stop",
            },
            Command {
                label: "Seek Forward",
                keys: ".",
                hint: "seek forward",
            },
            Command {
                label: "Seek Backward",
                keys: ",",
                hint: "seek backward",
            },
            Command {
                label: "Volume Up",
                keys: "+",
                hint: "volume up",
            },
            Command {
                label: "Volume Down",
                keys: "-",
                hint: "volume down",
            },
            Command {
                label: "Mute",
                keys: "m",
                hint: "mute",
            },
            Command {
                label: "Toggle Mono",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                label: "Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                label: "Toggle Shuffle",
                keys: "S",
                hint: "shuffle",
            },
            Command {
                label: "Toggle Favourite",
                keys: "f",
                hint: "toggle favourite",
            },
            Command {
                label: "Love / Un-love on Last.fm",
                keys: "*",
                hint: "love last.fm",
            },
            Command {
                label: "Like on Spotify",
                keys: "L",
                hint: "like spotify",
            },
            Command {
                label: "Add Live Track to Spotify",
                keys: "Alt+l",
                hint: "add to spotify",
            },
            Command {
                label: "Toggle Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                label: "Search This List",
                keys: "/",
                hint: "search this list",
            },
            Command {
                label: "Search Library",
                keys: "Alt+/",
                hint: "search library",
            },
            Command {
                label: "Library Categories",
                keys: "Alt+.",
                hint: "library lists",
            },
            Command {
                label: "Next Grouping",
                keys: "]",
                hint: "library next grouping",
            },
            Command {
                label: "Previous Grouping",
                keys: "[",
                hint: "library previous grouping",
            },
            Command {
                label: "Queue",
                keys: "Alt+q",
                hint: "queue",
            },
            Command {
                label: "YouTube Search",
                keys: "Alt+y",
                hint: "youtube",
            },
            Command {
                label: "Spotify",
                keys: "Alt+s",
                hint: "spotify",
            },
            Command {
                label: "Spotify Device: Next",
                keys: "",
                hint: "spotify next",
            },
            Command {
                label: "Spotify Device: Previous",
                keys: "",
                hint: "spotify previous",
            },
            Command {
                label: "Spotify Device: Shuffle",
                keys: "",
                hint: "spotify shuffle",
            },
            Command {
                label: "Spotify Device: Repeat",
                keys: "",
                hint: "spotify repeat",
            },
            Command {
                label: "Fetch Lyrics",
                keys: "l",
                hint: "fetch lyrics",
            },
            Command {
                label: "Clear Queue",
                keys: "D",
                hint: "clear queue",
            },
            Command {
                label: "Multiselect",
                keys: "v",
                hint: "multiselect",
            },
            Command {
                label: "Multiselect Up",
                keys: "Shift+Up",
                hint: "multiselect up",
            },
            Command {
                label: "Multiselect Down",
                keys: "Shift+Down",
                hint: "multiselect down",
            },
            Command {
                label: "Add to Queue",
                keys: "a",
                hint: "add to queue",
            },
            Command {
                label: "Add to Playlist",
                keys: "A",
                hint: "add to playlist",
            },
            Command {
                label: "Delete from List",
                keys: "x",
                hint: "delete from list",
            },
            Command {
                label: "Jump to End",
                keys: "G",
                hint: "jump to end",
            },
            Command {
                label: "Edit Metadata",
                keys: "e",
                hint: "edit metadata",
            },
            Command {
                label: "Focus Next Pane",
                keys: "Tab",
                hint: "focus pane forward",
            },
            Command {
                label: "Focus Previous Pane",
                keys: "Shift+Tab",
                hint: "focus pane back",
            },
            Command {
                label: "Cover Grid",
                keys: "V",
                hint: "cover grid",
            },
            Command {
                label: "Zen Mode",
                keys: "z",
                hint: "zen mode",
            },
            Command {
                label: "Cycle Theme",
                keys: "Alt+T",
                hint: "cycle theme",
            },
            Command {
                label: "Cycle Sort",
                keys: "Alt+S",
                hint: "cycle sort",
            },
            Command {
                label: "Podcasts",
                keys: "Alt+p",
                hint: "podcasts",
            },
            Command {
                label: "Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                label: "Equalizer",
                keys: "Alt+e",
                hint: "equalizer",
            },
            Command {
                label: "Sleep Timer",
                keys: "Alt+z",
                hint: "sleep timer",
            },
            Command {
                label: "Theme Picker",
                keys: "Alt+c",
                hint: "theme picker",
            },
            Command {
                label: "About",
                keys: "Alt+a",
                hint: "about",
            },
            Command {
                label: "Notifications",
                keys: "Alt+n",
                hint: "notifications",
            },
            Command {
                label: "Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                label: "Visualizer Preset",
                keys: "Alt+v",
                hint: "visualizer preset",
            },
            Command {
                label: "Quit",
                keys: "q",
                hint: "quit",
            },
            Command {
                label: "Quit Daemon",
                keys: "Q/Ctrl+Q",
                hint: "quit daemon",
            },
            Command {
                label: "Help",
                keys: "?",
                hint: "help",
            },
            Command {
                label: "Toggle Help Bar",
                keys: "Ctrl+h",
                hint: "toggle help bar",
            },
            Command {
                label: "Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                label: "Setup Services",
                keys: "Alt+x",
                hint: "setup",
            },
            Command {
                label: "Radio Browser",
                keys: "Alt+r",
                hint: "radio browse",
            },
            Command {
                label: "Play Stream URL",
                keys: "Alt+o",
                hint: "play stream url",
            },
        ]
    }
}

/// Group headings and how many rows each one owns, in list order.
///
/// The counts are a prefix sum the renderer walks, so they are checked against
/// the table rather than trusted: a command past the last count is a row the
/// palette never draws.
pub const COMMAND_GROUPS: &[(&str, usize)] = &[
    ("Playback", 14),
    ("Library & Queue", 25),
    ("View & Overlays", 15),
    ("System", 8),
];
