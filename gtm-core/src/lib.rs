// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Library root: re-exports all public types and Result<T>
//
// This is free software released under the GPL-3.0 license.

pub mod client;
pub mod fsm;
pub mod ipc;
pub mod log;
pub mod paths;
pub mod spotify;
pub mod state;
pub mod track;
pub mod tripwire;
pub mod validate;
pub mod wire;

pub use ipc::MetadataPatch;
pub use paths::{
    is_termux, resolve_command_socket, resolve_pid_file, resolve_pulse_socket, termux_music_dirs,
};
pub use spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
pub use state::{
    CoreError, CrossfadeConfig, DaemonState, Easing, EqBand, ReverbConfig, EQ_FREQUENCIES,
};
pub use track::{LrcData, LrcLine, Playlist, StreamInfo, TrackInfo, YTSearchResult};

pub type Result<T> = std::result::Result<T, CoreError>;
