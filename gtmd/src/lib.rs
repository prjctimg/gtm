// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon library root: re-exports all daemon submodules
//
// This is free software released under the GPL-3.0 license.

pub mod config;
pub mod cover_art;
pub mod daemon;
pub mod deezer;
pub mod library;
pub mod lyrics;
pub mod metadata_cleaner;
pub mod queue;
pub mod spotify;
pub mod tags;
pub mod youtube;

pub use config::DaemonConfig;
pub use daemon::Daemon;
