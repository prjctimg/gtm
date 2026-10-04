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
                label: "Mute: Toggle",
                keys: "m",
                hint: "mute",
            },
            Command {
                label: "Mono: Toggle",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                label: "Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                label: "Shuffle Library",
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
                label: "Add live track to Spotify",
                keys: "Alt+L",
                hint: "add to spotify",
            },
            Command {
                label: "Toggle Last.fm Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                label: "Search",
                keys: "/",
                hint: "search this list",
            },
            Command {
                label: "Search Library",
                keys: "Alt+/",
                hint: "search lib",
            },
            Command {
                label: "Library Categories",
                keys: "Alt+.",
                hint: "library lists",
            },
            Command {
                label: "Library: Next Grouping",
                keys: "Tab",
                hint: "library grouping",
            },
            Command {
                label: "Library: Previous Grouping",
                keys: "Shift+Tab",
                hint: "library grouping",
            },
            Command {
                label: "Queue",
                keys: "Alt+Q",
                hint: "queue",
            },
            Command {
                label: "YouTube Search",
                keys: "Alt+Y",
                hint: "youtube",
            },
            Command {
                label: "Spotify",
                keys: "Alt+S",
                hint: "spotify",
            },
            Command {
                label: "Spotify Next",
                keys: "",
                hint: "spotify next",
            },
            Command {
                label: "Spotify Previous",
                keys: "",
                hint: "spotify previous",
            },
            Command {
                label: "Spotify Shuffle",
                keys: "",
                hint: "spotify shuffle",
            },
            Command {
                label: "Spotify Repeat",
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
                keys: "]",
                hint: "focus next pane",
            },
            Command {
                label: "Focus Previous Pane",
                keys: "[",
                hint: "focus previous pane",
            },
            Command {
                label: "Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                label: "Equalizer",
                keys: "Alt+E",
                hint: "eq",
            },
            Command {
                label: "Sleep Timer",
                keys: "Alt+Z",
                hint: "sleeptimer",
            },
            Command {
                label: "Theme",
                keys: "Alt+C",
                hint: "themepicker",
            },
            Command {
                label: "About",
                keys: "Alt+A",
                hint: "about",
            },
            Command {
                label: "Notifications",
                keys: "Alt+N",
                hint: "notifications",
            },
            Command {
                label: "Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                label: "Visualizer Preset",
                keys: "Alt+V",
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
                label: "Toggle Help",
                keys: "?",
                hint: "toggle help",
            },
            Command {
                label: "Hide Help Bar",
                keys: "Ctrl+H",
                hint: "hide help bar",
            },
            Command {
                label: "Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                label: "Setup Services",
                keys: "Alt+X",
                hint: "setup",
            },
            Command {
                label: "Radio Browser",
                keys: "Alt+R",
                hint: "radio browse",
            },
            Command {
                label: "Play Stream URL",
                keys: "Alt+O",
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
    ("View & Overlays", 10),
    ("System", 8),
];
