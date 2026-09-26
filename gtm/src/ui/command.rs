// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Command palette entries
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub struct Command {
    pub icon: &'static str,
    pub keys: &'static str,
    pub hint: &'static str,
}

pub struct CommandPalette;

impl CommandPalette {
    pub fn commands(icon_style: &str) -> &'static [Command] {
        if icon_style == "mdi" {
            Self::commands_mdi()
        } else {
            Self::commands_emoji()
        }
    }

    pub(crate) fn commands_mdi() -> &'static [Command] {
        &[
            Command {
                icon: "\u{f04ba} Play/Pause",
                keys: "Space",
                hint: "play/pause",
            },
            Command {
                icon: "\u{f04ad} Next Track",
                keys: "n",
                hint: "next track",
            },
            Command {
                icon: "\u{f04a8} Prev Track",
                keys: "p",
                hint: "prev track",
            },
            Command {
                icon: "\u{f04cd} Stop",
                keys: "s",
                hint: "stop",
            },
            Command {
                icon: "\u{f04e2} Seek Forward",
                keys: ".",
                hint: "seek forward",
            },
            Command {
                icon: "\u{f04e0} Seek Backward",
                keys: ",",
                hint: "seek backward",
            },
            Command {
                icon: "\u{f057e} Volume Up",
                keys: "+",
                hint: "volume up",
            },
            Command {
                icon: "\u{f057d} Volume Down",
                keys: "-",
                hint: "volume down",
            },
            Command {
                icon: "\u{f0580} Mute: Toggle",
                keys: "m",
                hint: "mute",
            },
            Command {
                icon: "\u{f04ab} Mono: Toggle",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                icon: "\u{f0577} Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                icon: "\u{f0578} Shuffle Library",
                keys: "S",
                hint: "shuffle",
            },
            Command {
                icon: "\u{f0493} Toggle Favourite",
                keys: "f",
                hint: "toggle favourite",
            },
            Command {
                icon: "\u{f04a6} Love / Un-love on Last.fm",
                keys: "*",
                hint: "love last.fm",
            },
            Command {
                icon: "\u{f04ec} Toggle Last.fm Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                icon: "\u{f057a} Search",
                keys: "/",
                hint: "search this list",
            },
            Command {
                icon: "\u{f057a} Search Library",
                keys: "Alt+/",
                hint: "search lib",
            },
            Command {
                icon: "\u{f056e} Queue",
                keys: "Alt+Q",
                hint: "queue",
            },
            Command {
                icon: "\u{f16a} YouTube Search",
                keys: "Alt+Y",
                hint: "youtube",
            },
            Command {
                icon: "\u{f04c7} Spotify",
                keys: "Alt+S",
                hint: "spotify",
            },
            Command {
                icon: "\u{f1dd} Fetch Lyrics",
                keys: "l",
                hint: "fetch lyrics",
            },
            Command {
                icon: "\u{f156} Clear Queue",
                keys: "D",
                hint: "clear queue",
            },
            Command {
                icon: "\u{f285} Multiselect",
                keys: "v",
                hint: "multiselect",
            },
            Command {
                icon: "\u{f285} Multiselect Up",
                keys: "Shift+Up",
                hint: "multiselect up",
            },
            Command {
                icon: "\u{f285} Multiselect Down",
                keys: "Shift+Down",
                hint: "multiselect down",
            },
            Command {
                icon: "\u{f055e} Add to Queue",
                keys: "a",
                hint: "add to queue",
            },
            Command {
                icon: "\u{f055e} Add to Playlist",
                keys: "A",
                hint: "add to playlist",
            },
            Command {
                icon: "\u{f156} Delete from List",
                keys: "x",
                hint: "delete from list",
            },
            Command {
                icon: "\u{f045d} Jump to End",
                keys: "G",
                hint: "jump to end",
            },
            Command {
                icon: "\u{f0493} Edit Metadata",
                keys: "e",
                hint: "edit metadata",
            },
            Command {
                icon: "\u{f0493} Tab Cycle",
                keys: "Tab",
                hint: "tab cycle",
            },
            Command {
                icon: "\u{f0493} Prev Tab",
                keys: "Shift+Tab",
                hint: "prev tab",
            },
            Command {
                icon: "\u{f0493} Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                icon: "\u{f0570} Equalizer",
                keys: "Alt+E",
                hint: "eq",
            },
            Command {
                icon: "\u{f04b2} Sleep Timer",
                keys: "Alt+Z",
                hint: "sleeptimer",
            },
            Command {
                icon: "\u{f0493} Theme",
                keys: "Alt+C",
                hint: "themepicker",
            },
            Command {
                icon: "\u{f051d} Notifications",
                keys: "Alt+N",
                hint: "notifications",
            },
            Command {
                icon: "\u{f0493} Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                icon: "\u{f0570} Visualizer: Toggle",
                keys: "Ctrl+V",
                hint: "visualizer",
            },
            Command {
                icon: "\u{f0570} Visualizer Preset",
                keys: "Alt+V",
                hint: "visualizer preset",
            },
            Command {
                icon: "\u{f04db} Quit",
                keys: "q",
                hint: "quit",
            },
            Command {
                icon: "\u{f04db} Quit Daemon",
                keys: "Q/Ctrl+Q",
                hint: "quit daemon",
            },
            Command {
                icon: "\u{f051d} Toggle Help",
                keys: "?",
                hint: "toggle help",
            },
            Command {
                icon: "\u{f051d} Hide Help Bar",
                keys: "Ctrl+H",
                hint: "hide help bar",
            },
            Command {
                icon: "\u{f0493} Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                icon: "\u{f0493} Setup Services",
                keys: "Alt+X",
                hint: "setup",
            },
            Command {
                icon: "\u{f043b} Radio Browser",
                keys: "Alt+R",
                hint: "radio browse",
            },
            Command {
                icon: "\u{f056d} Play Stream URL",
                keys: "Alt+O",
                hint: "play stream url",
            },
        ]
    }

    pub(crate) fn commands_emoji() -> &'static [Command] {
        &[
            Command {
                icon: "\u{25b6}\u{fe0f} Play/Pause",
                keys: "Space",
                hint: "play/pause",
            },
            Command {
                icon: "\u{23ed}\u{fe0f} Next Track",
                keys: "n",
                hint: "next track",
            },
            Command {
                icon: "\u{23ee}\u{fe0f} Prev Track",
                keys: "p",
                hint: "prev track",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Stop",
                keys: "s",
                hint: "stop",
            },
            Command {
                icon: "\u{23e9}\u{fe0f} Seek Forward",
                keys: ".",
                hint: "seek forward",
            },
            Command {
                icon: "\u{23ea}\u{fe0f} Seek Backward",
                keys: ",",
                hint: "seek backward",
            },
            Command {
                icon: "\u{1f50a} Volume Up",
                keys: "+",
                hint: "volume up",
            },
            Command {
                icon: "\u{1f509} Volume Down",
                keys: "-",
                hint: "volume down",
            },
            Command {
                icon: "\u{1f507} Mute: Toggle",
                keys: "m",
                hint: "mute",
            },
            Command {
                icon: "\u{1f508} Mono: Toggle",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                icon: "\u{1f501} Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                icon: "\u{1f500} Shuffle Library",
                keys: "S",
                hint: "shuffle",
            },
            Command {
                icon: "\u{2764}\u{fe0f} Toggle Favourite",
                keys: "f",
                hint: "toggle favourite",
            },
            Command {
                icon: "\u{1f49e} Love / Un-love on Last.fm",
                keys: "*",
                hint: "love last.fm",
            },
            Command {
                icon: "\u{1f504} Toggle Last.fm Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                icon: "\u{1f50d} Search",
                keys: "/",
                hint: "search this list",
            },
            Command {
                icon: "\u{1f50e} Search Library",
                keys: "Alt+/",
                hint: "search lib",
            },
            Command {
                icon: "\u{1f4cb} Queue",
                keys: "Alt+Q",
                hint: "queue",
            },
            Command {
                icon: "\u{25b6}\u{fe0f} YouTube Search",
                keys: "Alt+Y",
                hint: "youtube",
            },
            Command {
                icon: "\u{1f3b5} Spotify",
                keys: "Alt+S",
                hint: "spotify",
            },
            Command {
                icon: "\u{1f4dd} Fetch Lyrics",
                keys: "l",
                hint: "fetch lyrics",
            },
            Command {
                icon: "\u{1f5d1} Clear Queue",
                keys: "D",
                hint: "clear queue",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect",
                keys: "v",
                hint: "multiselect",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect Up",
                keys: "Shift+Up",
                hint: "multiselect up",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect Down",
                keys: "Shift+Down",
                hint: "multiselect down",
            },
            Command {
                icon: "\u{2795} Add to Queue",
                keys: "a",
                hint: "add to queue",
            },
            Command {
                icon: "\u{1f4dc} Add to Playlist",
                keys: "A",
                hint: "add to playlist",
            },
            Command {
                icon: "\u{274c} Delete from List",
                keys: "x",
                hint: "delete from list",
            },
            Command {
                icon: "\u{2b07}\u{fe0f} Jump to End",
                keys: "G",
                hint: "jump to end",
            },
            Command {
                icon: "\u{270f}\u{fe0f} Edit Metadata",
                keys: "e",
                hint: "edit metadata",
            },
            Command {
                icon: "\u{27a1}\u{fe0f} Tab Cycle",
                keys: "Tab",
                hint: "tab cycle",
            },
            Command {
                icon: "\u{2b05}\u{fe0f} Prev Tab",
                keys: "Shift+Tab",
                hint: "prev tab",
            },
            Command {
                icon: "\u{2699}\u{fe0f} Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                icon: "\u{1f39a} Equalizer",
                keys: "Alt+E",
                hint: "eq",
            },
            Command {
                icon: "\u{23f0}\u{fe0f} Sleep Timer",
                keys: "Alt+Z",
                hint: "sleeptimer",
            },
            Command {
                icon: "\u{1f3a8} Theme",
                keys: "Alt+C",
                hint: "themepicker",
            },
            Command {
                icon: "\u{2139}\u{fe0f} About",
                keys: "Alt+A",
                hint: "about",
            },
            Command {
                icon: "\u{1f514} Notifications",
                keys: "Alt+N",
                hint: "notifications",
            },
            Command {
                icon: "\u{1f3a8} Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                icon: "\u{1f3b6} Visualizer: Toggle",
                keys: "Ctrl+V",
                hint: "visualizer",
            },
            Command {
                icon: "\u{1f3b6} Visualizer Preset",
                keys: "Alt+V",
                hint: "visualizer preset",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Quit",
                keys: "q",
                hint: "quit",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Quit Daemon",
                keys: "Q/Ctrl+Q",
                hint: "quit daemon",
            },
            Command {
                icon: "\u{2753} Toggle Help",
                keys: "?",
                hint: "toggle help",
            },
            Command {
                icon: "\u{1f6ab} Hide Help Bar",
                keys: "Ctrl+H",
                hint: "hide help bar",
            },
            Command {
                icon: "\u{1fa7a} Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                icon: "\u{2699}\u{fe0f} Setup Services",
                keys: "Alt+X",
                hint: "setup",
            },
            Command {
                icon: "\u{1f3f7}\u{fe0f} Radio Browser",
                keys: "Alt+R",
                hint: "radio browse",
            },
        ]
    }
}

pub const COMMAND_GROUPS: &[(&str, usize)] = &[
    ("Playback", 12),
    ("Library & Queue", 15),
    ("View & Overlays", 11),
    ("System", 7),
];
