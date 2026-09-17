// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// gtm library crate: TUI + CLI modules exposed so integration tests under
// `gtm/tests/` can exercise them directly. The `gtm` binary is a thin entry
// point over this library. The core, audio, MPRIS, and daemon subsystems live
// as module trees (`shared`, `audio`, `mpris`, `gtmd`) inside this crate.
// With `--no-default-features` only the bundled daemon binary
// (src/daemon_main.rs) is built.
//
// This is free software released under the GPL-3.0 license.

pub mod audio;
pub mod gtmd;
#[cfg(feature = "mpris")]
pub mod mpris;
pub mod shared;

pub use gtmd::run;
pub use gtmd::{Daemon, DaemonArgs, DaemonConfig};

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
