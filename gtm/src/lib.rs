// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// gtm library crate: TUI + CLI modules exposed so integration tests under
// `gtm/tests/` can exercise them directly. The `gtm` binary is a thin
// entry point over this library. With `--no-default-features` only the
// bundled daemon binary (src/daemon_main.rs) is built.
//
// This is free software released under the GPL-3.0 license.

#[cfg(feature = "tui")]
pub mod app;
#[cfg(feature = "tui")]
pub mod cli;
#[cfg(feature = "tui")]
pub mod extensions;
#[cfg(feature = "tui")]
pub mod footer;
#[cfg(feature = "tui")]
pub mod keymap;
#[cfg(feature = "tui")]
pub mod mouse;
#[cfg(feature = "tui")]
pub mod oauth;
#[cfg(feature = "tui")]
pub mod picker;
#[cfg(feature = "tui")]
pub mod progress;
#[cfg(feature = "tui")]
pub mod reactive;
#[cfg(feature = "tui")]
pub mod theme;
#[cfg(feature = "tui")]
pub mod ui;
#[cfg(feature = "tui")]
pub mod visualizer;
