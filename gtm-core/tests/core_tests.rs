// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Integration tests for gtm-core: serde, wire, state machine, and invariants
//
// This is free software released under the GPL-3.0 license.

use gtm_core::Result;
use gtm_core::global::{
    CrossfadeConfig, DaemonState, Image, PlaybackStatus, RepeatMode, ThemeMode, UIMode, YTFilter,
};
use gtm_core::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction};
use gtm_core::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use gtm_core::track::{LrcData, LrcLine, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use gtm_core::wire::{decode, encode};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_track() -> TrackInfo {
    TrackInfo {
        id: 1,
        path: "/music/song.mp3".into(),
        title: "Test Song".into(),
        artist: "Test Artist".into(),
        album: "Test Album".into(),
        duration: 240.0,
        track_number: Some(1),
        genre: "Rock".into(),
        year: Some(2024),
        bitrate: Some(320),
        samplerate: Some(44100),
        hash: "abc123".into(),
        cover_path: Some("/covers/test.jpg".into()),
        favourite: false,
        ..Default::default()
    }
}

fn sample_state() -> DaemonState {
    let mut s = DaemonState::new();
    s.queue = vec![sample_track()];
    s.queue_cursor = 0;
    s
}

// ---------------------------------------------------------------------------
// Serde round-trips
// ---------------------------------------------------------------------------

macro_rules! roundtrip {
    ($name:ident, $ty:ty, $val:expr) => {
        #[test]
        fn $name() {
            let val: $ty = $val;
            // JSON
            let json = serde_json::to_string(&val).unwrap();
            let de: $ty = serde_json::from_str(&json).unwrap();
            // For structs we can't directly compare fn equality; compare debug
            assert_eq!(
                format!("{:?}", val),
                format!("{:?}", de),
                "JSON round-trip failed"
            );
            // Bincode
            let bin = bincode::serialize(&val).unwrap();
            let de2: $ty = bincode::deserialize(&bin).unwrap();
            assert_eq!(
                format!("{:?}", val),
                format!("{:?}", de2),
                "bincode round-trip failed"
            );
        }
    };
}

roundtrip!(track_info_roundtrip, TrackInfo, sample_track());
roundtrip!(
    playlist_roundtrip,
    Playlist,
    Playlist {
        id: 1,
        name: "Favourites".into(),
        created_at: "2024-01-01T00:00:00Z".into(),
        track_count: 10,
    }
);
roundtrip!(
    lrc_line_roundtrip,
    LrcLine,
    LrcLine {
        timestamp: 12.5,
        text: "hello".into(),
    }
);
roundtrip!(
    lrc_data_roundtrip,
    LrcData,
    LrcData {
        title: Some("Song".into()),
        artist: Some("Artist".into()),
        album: Some("Album".into()),
        lines: vec![LrcLine {
            timestamp: 0.0,
            text: "intro".into(),
        }],
    }
);
roundtrip!(
    yt_result_roundtrip,
    YTSearchResult,
    YTSearchResult {
        id: "abc".into(),
        title: "Test Vid".into(),
        url: "https://youtube.com/watch?v=abc".into(),
        channel: "TestChannel".into(),
        artist: Some("TestArtist".into()),
        priority: 2,
        duration: 120.0,
        views: 1000,
        thumbnail: Some("https://img.youtube.com/vi/abc/default.jpg".into()),
        is_playlist: false,
    }
);
roundtrip!(
    stream_info_roundtrip,
    StreamInfo,
    StreamInfo {
        url: "https://example.com/stream".into(),
        title: "Stream".into(),
        ext: "mp3".into(),
        duration: 300.0,
    }
);
roundtrip!(
    crossfade_config_roundtrip,
    CrossfadeConfig,
    CrossfadeConfig {
        enabled: true,
        duration_secs: 8,
    }
);
roundtrip!(daemon_state_roundtrip, DaemonState, sample_state());
roundtrip!(
    image_roundtrip,
    Image,
    Image {
        data: vec![0, 1, 2],
        mime: "image/jpeg".into(),
        width: 100,
        height: 100,
    }
);

// ---------------------------------------------------------------------------
// IPC: cmd_name round-trips (parse_cmd is the canonical deserialization path)
// ---------------------------------------------------------------------------

#[test]
fn daemon_req_cmd_name_roundtrip() {
    let reqs: Vec<DaemonReq> = vec![
        DaemonReq::Play {
            path: "/m/s.mp3".into(),
            start_pos: 0.0,
        },
        DaemonReq::PlayPause,
        DaemonReq::Pause,
        DaemonReq::Stop,
        DaemonReq::Next,
        DaemonReq::Prev,
        DaemonReq::Seek {
            position_secs: 10.0,
        },
        DaemonReq::SetVolume { volume: 80 },
        DaemonReq::GetVolume,
        DaemonReq::ToggleShuffle,
        DaemonReq::ToggleMute,
        DaemonReq::GetStatus,
        DaemonReq::CheckHealth,
        DaemonReq::Ping,
        DaemonReq::Quit,
    ];
    for req in &reqs {
        let cmd = req.cmd_name();
        let params = serde_json::to_value(req).unwrap();
        let de = DaemonReq::parse_cmd(cmd, params).unwrap();
        assert_eq!(req.cmd_name(), de.cmd_name(), "roundtrip failed for {cmd}");
    }
}

#[test]
fn daemon_req_parse_cmd_unknown_cmd() {
    let result = DaemonReq::parse_cmd("totally_unknown", serde_json::json!({}));
    assert!(result.is_err());
}

#[test]
fn daemon_req_parse_cmd_play() {
    let params = serde_json::json!({"path": "/music/song.mp3", "start_pos": 0.0});
    let req = DaemonReq::parse_cmd("play", params).unwrap();
    assert_eq!(req.cmd_name(), "play");
    match req {
        DaemonReq::Play { path, start_pos } => {
            assert_eq!(path, "/music/song.mp3");
            assert_eq!(start_pos, 0.0);
        }
        other => panic!("expected Play, got {other:?}"),
    }
}

#[test]
fn daemon_req_parse_cmd_unit_variants() {
    for (cmd, expected) in [
        ("play_pause", "play_pause"),
        ("pause", "pause"),
        ("stop", "stop"),
        ("next", "next"),
        ("prev", "prev"),
        ("get_volume", "get_volume"),
        ("toggle_shuffle", "toggle_shuffle"),
        ("toggle_mute", "toggle_mute"),
        ("get_status", "get_status"),
        ("check_health", "check_health"),
        ("ping", "ping"),
        ("quit", "quit"),
    ] {
        let req = DaemonReq::parse_cmd(cmd, serde_json::json!({})).unwrap();
        assert_eq!(req.cmd_name(), expected);
    }
}

#[test]
fn daemon_req_parse_cmd_spotify_variants() {
    let cases: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "spotify_set_token",
            serde_json::json!({ "token": "BQCabc" }),
            "spotify_set_token",
        ),
        ("spotify_clear", serde_json::json!({}), "spotify_clear"),
        ("spotify_status", serde_json::json!({}), "spotify_status"),
        ("spotify_sync", serde_json::json!({}), "spotify_sync"),
        (
            "spotify_playlists",
            serde_json::json!({}),
            "spotify_playlists",
        ),
        (
            "spotify_playlist_tracks",
            serde_json::json!({ "id": "37i9dQZEVX" }),
            "spotify_playlist_tracks",
        ),
        (
            "spotify_resolve",
            serde_json::json!({ "playlist_id": "37i9dQZEVX", "track_index": 3 }),
            "spotify_resolve",
        ),
    ];
    for (cmd, params, expected) in cases {
        let req = DaemonReq::parse_cmd(cmd, params).unwrap();
        assert_eq!(req.cmd_name(), expected);
        match req {
            DaemonReq::SpotifySetToken { token } => assert_eq!(token, "BQCabc"),
            DaemonReq::SpotifyPlaylistTracks { id } => assert_eq!(id, "37i9dQZEVX"),
            DaemonReq::SpotifyResolve {
                playlist_id,
                track_index,
            } => {
                assert_eq!(playlist_id, "37i9dQZEVX");
                assert_eq!(track_index, 3);
            }
            _ => {}
        }
    }
}

#[test]
fn daemon_res_spotify_wire_roundtrip() {
    let status = SpotifyStatus {
        linked: true,
        user: Some("test-user".into()),
        premium: true,
        playing: false,
        device: Some("Test Speaker".into()),
        playlists: 2,
        tracks: 5,
        error: None,
    };
    let playlist = SpotifyPlaylist {
        id: "37i9dQZEVX".into(),
        name: "Test Mix".into(),
        owner: "spotify".into(),
        tracks: vec![SpotifyTrack {
            index: 0,
            name: "Song".into(),
            artists: "Artist".into(),
            album: Some("Album".into()),
            duration_ms: Some(240000),
            uri: None,
        }],
    };
    let cases: Vec<(&str, DaemonRes)> = vec![
        (
            "spotify_status",
            DaemonRes::SpotifyStatusRes {
                status: status.clone(),
            },
        ),
        (
            "spotify_playlists",
            DaemonRes::SpotifyPlaylistsRes {
                playlists: vec![playlist.clone()],
            },
        ),
        (
            "spotify_playlist_tracks",
            DaemonRes::SpotifyTracksRes {
                tracks: playlist.tracks.clone(),
            },
        ),
    ];
    for (cmd, res) in cases {
        let expected = format!("{:?}", res);
        let wire = res.to_wire(1);
        let back = DaemonRes::from_wire(cmd, &wire);
        assert_eq!(
            expected,
            format!("{:?}", back),
            "round-trip failed for {cmd}"
        );
    }
}

// ---------------------------------------------------------------------------
// IPC: wire encode/decode round-trips (bincode via the wire module)
// ---------------------------------------------------------------------------

macro_rules! wire_event_roundtrip {
    ($name:ident, $event:expr) => {
        #[test]
        fn $name() {
            let events = vec![$event];
            let buf = encode(&events).unwrap();
            let (decoded, consumed) = decode(&buf).unwrap().unwrap();
            assert_eq!(decoded.len(), 1);
            assert_eq!(consumed as usize, buf.len());
            assert_eq!(format!("{:?}", events[0]), format!("{:?}", decoded[0]));
        }
    };
}

wire_event_roundtrip!(
    wire_event_playback_started,
    DaemonEvent::PlaybackStarted {
        track: sample_track(),
        auto_advanced: false,
        time_pos: 0.0,
        duration: 240.0,
    }
);
wire_event_roundtrip!(
    wire_event_playback_paused,
    DaemonEvent::PlaybackPaused { time_pos: 0.0 }
);
wire_event_roundtrip!(wire_event_track_ended, DaemonEvent::TrackEnded);
wire_event_roundtrip!(
    wire_event_volume_changed,
    DaemonEvent::VolumeChanged { volume: 50 }
);

// ---------------------------------------------------------------------------
// IPC: DaemonRes serde round-trips (internally tagged, JSON-only)
// ---------------------------------------------------------------------------

#[test]
fn daemon_res_json_roundtrip() {
    let ress: Vec<DaemonRes> = vec![
        DaemonRes::Ok,
        DaemonRes::Pong,
        DaemonRes::Error {
            message: "fail".into(),
        },
    ];
    for res in &ress {
        let json = serde_json::to_value(res).unwrap();
        let de: DaemonRes = serde_json::from_value(json).unwrap();
        assert_eq!(format!("{:?}", res), format!("{:?}", de));
    }
}

// ---------------------------------------------------------------------------
// IPC: QueueAction / LibraryAction serde JSON round-trips (internally tagged)
// ---------------------------------------------------------------------------

#[test]
fn queue_action_json_roundtrip() {
    let actions: Vec<QueueAction> = vec![
        QueueAction::List,
        QueueAction::Clear,
        QueueAction::Add {
            paths: vec!["/m/s.mp3".into()],
            position: None,
        },
        QueueAction::Add {
            paths: vec!["/a.mp3".into(), "/b.mp3".into()],
            position: Some(2),
        },
    ];
    for action in &actions {
        let json = serde_json::to_value(action).unwrap();
        let de: QueueAction = serde_json::from_value(json).unwrap();
        assert_eq!(format!("{:?}", action), format!("{:?}", de));
    }
}

#[test]
fn library_action_json_roundtrip() {
    let actions: Vec<LibraryAction> = vec![
        LibraryAction::Scan {
            path: "/music".into(),
        },
        LibraryAction::GetTracks {
            filter: None,
            sort: None,
        },
        LibraryAction::GetPlaylists,
        LibraryAction::CreatePlaylist {
            name: "Favs".into(),
        },
        LibraryAction::DeletePlaylist { id: 1 },
        LibraryAction::AddToPlaylist {
            playlist_id: 1,
            track_ids: vec![1, 2],
        },
        LibraryAction::ImportM3u {
            path: "/m.m3u".into(),
        },
        LibraryAction::ExportM3u {
            playlist_id: 1,
            path: "/out.m3u".into(),
        },
        LibraryAction::SyncCovers,
        LibraryAction::SyncLyrics,
        LibraryAction::SyncMetadata { path: None },
        LibraryAction::SyncMetadata {
            path: Some("/music/track.mp3".into()),
        },
        LibraryAction::RemoveFromPlaylist {
            playlist_id: 1,
            track_id: 2,
        },
        LibraryAction::RemoveTrack { id: 1 },
    ];
    for action in &actions {
        let json = serde_json::to_value(action).unwrap();
        let de: LibraryAction = serde_json::from_value(json).unwrap();
        assert_eq!(format!("{:?}", action), format!("{:?}", de));
    }
}

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

#[test]
fn encode_decode_empty() {
    let buf = encode(&[]).unwrap();
    let (frame, consumed) = decode(&buf).unwrap().unwrap();
    assert!(frame.is_empty());
    assert_eq!(consumed, buf.len());
}

#[test]
fn encode_decode_one() {
    let events = vec![DaemonEvent::PlaybackPaused { time_pos: 0.0 }];
    let buf = encode(&events).unwrap();
    let (frame, consumed) = decode(&buf).unwrap().unwrap();
    assert_eq!(frame.len(), 1);
    assert!(matches!(frame[0], DaemonEvent::PlaybackPaused { .. }));
    assert_eq!(consumed, buf.len());
}

#[test]
fn encode_decode_multi() {
    let events = vec![
        DaemonEvent::PlaybackPaused { time_pos: 0.0 },
        DaemonEvent::VolumeChanged { volume: 50 },
        DaemonEvent::TrackEnded,
    ];
    let buf = encode(&events).unwrap();
    let (frame, consumed) = decode(&buf).unwrap().unwrap();
    assert_eq!(frame.len(), 3);
    assert_eq!(consumed, buf.len());
}

#[test]
fn decode_partial_buffer_returns_none() {
    let events = vec![DaemonEvent::PlaybackPaused { time_pos: 0.0 }];
    let buf = encode(&events).unwrap();
    // Truncate to only length prefix
    assert!(decode(&buf[..2]).unwrap().is_none());
    // Truncate to just past length prefix
    assert!(decode(&buf[..5]).unwrap().is_none());
}

#[test]
fn decode_truncated_data_returns_none() {
    let corrupted = vec![0u8, 0, 0, 5, 0xff, 0xff, 0xff];
    assert!(decode(&corrupted).unwrap().is_none());
}

#[test]
fn decode_corrupted_bincode_returns_error() {
    // Length says 4 bytes, but content is not valid bincode
    let bad = b"\x00\x00\x00\x04\xff\xff\xff\xff".to_vec();
    assert!(decode(&bad).is_err());
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

#[test]
fn state_transition_stop_play_pause_play_stop() {
    let mut s = sample_state();
    let track = sample_track();

    // Stopped -> Playing
    assert_eq!(s.status, PlaybackStatus::Stopped);
    s.play(track.clone()).unwrap();
    assert_eq!(s.status, PlaybackStatus::Playing);
    assert_eq!(s.current_track.as_ref().unwrap().id, 1);

    // Playing -> Paused
    s.pause().unwrap();
    assert_eq!(s.status, PlaybackStatus::Paused);

    // Paused -> Playing
    s.play(track.clone()).unwrap();
    assert_eq!(s.status, PlaybackStatus::Playing);

    // Playing -> Stopped
    s.stop().unwrap();
    assert_eq!(s.status, PlaybackStatus::Stopped);
    assert!(s.current_track.is_none());
    assert_eq!(s.time_pos, 0.0);
}

#[test]
fn state_transition_seek() {
    let mut s = sample_state();
    s.duration = 200.0;
    s.seek(50.0).unwrap();
    assert!((s.time_pos - 50.0).abs() < f64::EPSILON);
}

#[test]
fn state_transition_seek_clamped() {
    let mut s = sample_state();
    s.duration = 200.0;
    s.seek(999.0).unwrap();
    assert!((s.time_pos - 200.0).abs() < f64::EPSILON);

    s.seek(-10.0).unwrap();
    assert!((s.time_pos - 0.0).abs() < f64::EPSILON);
}

#[test]
fn state_transition_volume() {
    let mut s = sample_state();
    s.set_volume(75).unwrap();
    assert_eq!(s.volume, 75);
    assert!(!s.mute);
}

#[test]
fn state_transition_volume_clamped() {
    let mut s = sample_state();
    s.set_volume(200).unwrap();
    assert_eq!(s.volume, 100);
}

#[test]
fn state_transition_shuffle_toggle() {
    let mut s = sample_state();
    assert!(!s.shuffle);
    s.toggle_shuffle().unwrap();
    assert!(s.shuffle);
    s.toggle_shuffle().unwrap();
    assert!(!s.shuffle);
}

#[test]
fn state_transition_repeat_cycle() {
    let mut s = sample_state();
    s.cycle_repeat(RepeatMode::One).unwrap();
    assert_eq!(s.repeat, RepeatMode::One);
    s.cycle_repeat(RepeatMode::All).unwrap();
    assert_eq!(s.repeat, RepeatMode::All);
}

#[test]
fn state_transition_mute_toggle() {
    let mut s = sample_state();
    assert!(!s.mute);
    s.toggle_mute().unwrap();
    assert!(s.mute);
    s.toggle_mute().unwrap();
    assert!(!s.mute);
}

#[test]
fn state_transition_crossfade() {
    let mut s = sample_state();
    s.set_crossfade(true, 8).unwrap();
    assert!(s.crossfade.is_some());
    assert_eq!(s.crossfade.as_ref().unwrap().duration_secs, 8);

    s.set_crossfade(false, 0).unwrap();
    assert!(s.crossfade.is_none());
}

#[test]
fn state_transition_crossfade_clamped() {
    let mut s = sample_state();
    s.set_crossfade(true, 99).unwrap();
    assert_eq!(s.crossfade.as_ref().unwrap().duration_secs, 30);
}

#[test]
fn state_transition_advance_queue_one_time() {
    let mut s = sample_state();
    let t2 = TrackInfo {
        id: 2,
        path: "/music/song2.mp3".into(),
        ..sample_track()
    };
    let t3 = TrackInfo {
        id: 3,
        path: "/music/song3.mp3".into(),
        ..sample_track()
    };
    s.queue.push(t2);
    s.queue.push(t3);
    s.queue_cursor = 0;
    s.repeat = RepeatMode::All;

    // The queue is a one-time FIFO: advancing consumes the head and surfaces
    // the next pending entry.  sample_state() starts with track id 1.
    let next = s.advance_queue().unwrap().unwrap();
    assert_eq!(next.id, 2);
    assert_eq!(s.queue.len(), 2);
    assert_eq!(s.queue_cursor, 0);

    let next = s.advance_queue().unwrap().unwrap();
    assert_eq!(next.id, 3);
    assert_eq!(s.queue.len(), 1);

    // Exhausted queue -> None.
    assert!(s.advance_queue().unwrap().is_none());
    assert!(s.queue.is_empty());
}

#[test]
fn state_transition_advance_queue_empty() {
    let mut s = DaemonState::new();
    assert!(s.advance_queue().unwrap().is_none());
}

#[test]
fn state_transition_version_increments() {
    let mut s = sample_state();
    let v0 = s.version;
    s.play(sample_track()).unwrap();
    assert_eq!(s.version, v0 + 1);
    s.pause().unwrap();
    assert_eq!(s.version, v0 + 2);
}

// ---------------------------------------------------------------------------
// Event application
// ---------------------------------------------------------------------------

#[test]
fn apply_playback_started() {
    let mut s = DaemonState::new();
    s.apply_event(&DaemonEvent::PlaybackStarted {
        track: sample_track(),
        auto_advanced: false,
        time_pos: 0.0,
        duration: 240.0,
    });
    assert_eq!(s.status, PlaybackStatus::Playing);
    assert_eq!(s.current_track.as_ref().unwrap().id, 1);
    assert!((s.duration - 240.0).abs() < f64::EPSILON);
}

#[test]
fn apply_playback_paused() {
    let mut s = sample_state();
    s.apply_event(&DaemonEvent::PlaybackPaused { time_pos: 0.0 });
    assert_eq!(s.status, PlaybackStatus::Paused);
}

#[test]
fn apply_playback_stopped() {
    let mut s = sample_state();
    s.current_track = Some(sample_track());
    s.apply_event(&DaemonEvent::PlaybackStopped);
    assert_eq!(s.status, PlaybackStatus::Stopped);
    assert!(s.current_track.is_none());
    assert_eq!(s.time_pos, 0.0);
}

#[test]
fn apply_position_changed() {
    let mut s = sample_state();
    s.apply_event(&DaemonEvent::PositionChanged { time_pos: 42.5 });
    assert!((s.time_pos - 42.5).abs() < f64::EPSILON);
}

#[test]
fn apply_volume_changed() {
    let mut s = sample_state();
    s.apply_event(&DaemonEvent::VolumeChanged { volume: 80 });
    assert_eq!(s.volume, 80);
}

#[test]
fn apply_queue_changed() {
    let mut s = sample_state();
    let t2 = TrackInfo {
        id: 2,
        path: "/music/s2.mp3".into(),
        ..sample_track()
    };
    s.apply_event(&DaemonEvent::QueueChanged {
        queue: vec![t2.clone()],
        cursor: 0u64,
    });
    assert_eq!(s.queue.len(), 1);
    assert_eq!(s.queue[0].id, 2);
    assert_eq!(s.queue_cursor, 0u64);
}

#[test]
fn apply_repeat_mode_changed() {
    let mut s = sample_state();
    s.apply_event(&DaemonEvent::RepeatModeChanged {
        mode: RepeatMode::One,
    });
    assert_eq!(s.repeat, RepeatMode::One);
}

#[test]
fn apply_event_increments_version() {
    let mut s = sample_state();
    let v0 = s.version;
    s.apply_event(&DaemonEvent::PlaybackPaused { time_pos: 0.0 });
    assert_eq!(s.version, v0 + 1);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn crossfade_config_new_clamps() {
    let c = CrossfadeConfig::new(true, 40);
    assert_eq!(c.duration_secs, 30);
    let c = CrossfadeConfig::new(true, 5);
    assert_eq!(c.duration_secs, 5);
}

#[test]
fn daemon_state_new_defaults() {
    let s = DaemonState::new();
    assert_eq!(s.status, PlaybackStatus::Stopped);
    assert_eq!(s.volume, 100);
    assert_eq!(s.version, 0);
    assert!(s.queue.is_empty());
    assert_eq!(s.queue_cursor, 0);
    assert!(s.current_track.is_none());
}

#[test]
fn track_info_is_valid() {
    let t = sample_track();
    assert!(t.is_valid());
}

#[test]
fn track_info_invalid_empty_path() {
    let mut t = sample_track();
    t.path.clear();
    assert!(!t.is_valid());
}

#[test]
fn track_info_invalid_empty_hash() {
    let mut t = sample_track();
    t.hash.clear();
    assert!(!t.is_valid());
}

#[test]
fn track_info_invalid_negative_duration() {
    let mut t = sample_track();
    t.duration = -1.0;
    assert!(!t.is_valid());
}

#[test]
fn track_info_duration_formatted() {
    let mut t = sample_track();
    t.duration = 245.0; // 4:05
    assert_eq!(t.duration_formatted(), "4:05");

    t.duration = 3661.0; // 1:01:01
    assert_eq!(t.duration_formatted(), "1:01:01");
}

// ---------------------------------------------------------------------------
// Primitives & enums
// ---------------------------------------------------------------------------

#[test]
fn primitives_derive_traits() {
    // Compile-time check that Copy works
    let s = PlaybackStatus::Playing;
    let _s2 = s;
    let _ = format!("{:?}", s);

    let r = RepeatMode::Off;
    let _r2 = r;
    let _ = format!("{:?}", r);

    let t = ThemeMode::Dark;
    let _t2 = t;

    let u = UIMode::Normal;
    let _u2 = u;

    let yt = YTFilter::Song;
    let _yt2 = yt;
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[test]
fn malformed_json_returns_error() {
    let result: Result<TrackInfo> = serde_json::from_str("not valid json").map_err(Into::into);
    assert!(result.is_err());
}

#[test]
fn truncated_bincode_returns_error() {
    let track = sample_track();
    let full = bincode::serialize(&track).unwrap();
    let truncated = &full[..full.len() / 2];
    let result: std::result::Result<TrackInfo, _> = bincode::deserialize(truncated);
    assert!(result.is_err());
}

#[test]
fn empty_wire_frame() {
    let buf = encode(&[]).unwrap();
    let (frame, _) = decode(&buf).unwrap().unwrap();
    assert!(frame.is_empty());
}

#[test]
fn unknown_cmd_is_error() {
    let result = DaemonReq::parse_cmd("unknown_command", serde_json::json!({}));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

#[test]
fn check_invariants_passes_on_valid_state() {
    let s = sample_state();
    // Should not panic
    s.check_invariants();
}

#[test]
#[should_panic(expected = "volume 101 exceeds 100")]
fn check_invariants_volume_too_high() {
    let mut s = sample_state();
    s.volume = 101;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "out of bounds")]
fn check_invariants_queue_cursor_oob() {
    let mut s = sample_state();
    s.queue_cursor = 99;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "negative time_pos")]
fn check_invariants_negative_time_pos() {
    let mut s = sample_state();
    s.time_pos = -1.0;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "Playing but current_track is None")]
fn check_invariants_playing_no_track() {
    let mut s = sample_state();
    s.status = PlaybackStatus::Playing;
    s.current_track = None;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "crossfade enabled with duration_secs = 0")]
fn check_invariants_crossfade_zero_duration() {
    let mut s = sample_state();
    s.crossfade = Some(CrossfadeConfig {
        enabled: true,
        duration_secs: 0,
    });
    s.check_invariants();
}

// ---------------------------------------------------------------------------
// DaemonState default
// ---------------------------------------------------------------------------

#[test]
fn daemon_state_default_eq_new() {
    let a = DaemonState::new();
    let b = DaemonState::default();
    assert_eq!(a.version, b.version);
    assert_eq!(a.status, b.status);
    assert_eq!(a.volume, b.volume);
}
