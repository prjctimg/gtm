// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Library root: re-exports all public types and Result<T>
//
// This is free software released under the GPL-3.0 license.

pub mod client;
pub mod custom;
pub mod daemon;
pub mod fsm;
pub mod global;
pub mod ipc;
pub mod log;
pub mod paths;
pub mod playlist;
pub mod secret;
pub mod state;
pub mod track;
pub mod tripwire;
pub mod url;
pub mod validate;
pub mod wire;

// Each provider's wire types live under `providers`; re-exported here so the
// shared vocabulary stays flat for the rest of the client.
pub use crate::providers::charts as chart;
pub use crate::providers::podcast;
pub use crate::providers::radio;
pub use crate::providers::spotify;
pub use crate::providers::yt;

pub use crate::shared::custom::CustomRadioStation;
pub use chart::{ChartError, ChartPlaylist, ChartSource, ChartTrack};
pub use global::{
    CoreError, CrossfadeConfig, DEFAULT_SPEED, DaemonState, EQ_FREQUENCIES, EqBand, MAX_SPEED,
    MAX_VOLUME, MIN_SPEED, ReverbConfig, volume_from_ratio, volume_ratio,
};
pub use ipc::MetadataPatch;
pub use paths::{
    ensure_termux_pulse, is_termux, resolve_command_socket, resolve_pid_file, resolve_pulse_socket,
    termux_music_dirs,
};
pub use playlist::{M3u8Format, PlaylistFormat, PlaylistFormatKind, PlsFormat};
pub use podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};
pub use radio::RadioStation;
pub use spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
pub use track::{LrcData, LrcLine, Playlist, StreamInfo, TrackInfo, YTSearchResult};

pub type Result<T> = std::result::Result<T, CoreError>;
