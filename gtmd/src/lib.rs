// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon library root: re-exports all daemon submodules
//
// This is free software released under the GPL-3.0 license.

pub mod cleaner;
pub mod config;
pub mod cover;
pub mod daemon;
pub mod deezer;
pub mod library;
pub mod lyrics;
pub mod oauth;
pub mod queue;
pub mod spotify;
pub mod spotify_stream;
pub mod tags;
pub mod youtube;

pub use config::DaemonConfig;
pub use daemon::Daemon;
