// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// gtm library crate: TUI + CLI modules exposed so integration tests under
// `gtm/tests/` can exercise them directly. The `gtm` binary is a thin entry
// point over this library. The shared, audio, and MPRIS subsystems live as
// module trees (`shared`, `audio`, `mpris`) inside this crate; the daemon
// lives in the separate `gtmd` crate, which depends on this one for the IPC
// wire types, audio mixers, and MPRIS integration.
// With `--no-default-features` the MPRIS module (daemon-only) is excluded.
//
// This is free software released under the GPL-3.0 license.

pub mod audio;
#[cfg(feature = "mpris")]
pub mod mpris;
pub mod shared;

pub mod app;
pub mod cli;
pub mod extensions;
pub mod footer;
pub mod keymap;
pub mod mouse;
pub mod oauth;
pub mod picker;
pub mod progress;
pub mod reactive;
pub mod theme;
pub mod ui;
pub mod visualizer;
