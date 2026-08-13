// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Library root: re-exports all public types and Result<T>
//
// This is free software released under the GPL-3.0 license.

pub mod client;
pub mod ipc;
pub mod state;
pub mod state_machine;
pub mod track;
pub mod tripwire;
pub mod validate;
pub mod wire;

pub use state::{CoreError, CrossfadeConfig, DaemonState, Easing, EqBand, EQ_FREQUENCIES, ReverbConfig};
pub use track::{LrcData, LrcLine, Playlist, StreamInfo, TrackInfo, YTSearchResult};

pub type Result<T> = std::result::Result<T, CoreError>;
