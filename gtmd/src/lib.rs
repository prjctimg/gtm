// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon library root: re-exports all daemon submodules
//
// This is free software released under the GPL-3.0 license.

pub mod cleaner;
pub mod config;
pub mod cover;
pub mod daemon;
pub mod deezer;
pub mod lastfm;
pub mod library;
pub mod lyrics;
pub mod oauth;
pub mod queue;
pub mod spotify;
pub mod stream;
pub mod tags;
#[cfg(feature = "youtube")]
pub mod youtube;

pub use config::DaemonConfig;
pub use daemon::Daemon;
