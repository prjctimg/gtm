// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Integration tests for shared: serde, wire, state machine, and invariants
//
// This is free software released under the GPL-3.0 license.

use gtm::shared::Result;
use gtm::shared::global::{
    CrossfadeConfig, DaemonState, Image, PlaybackStatus, RepeatMode, ThemeMode, UIMode, YTFilter,
};
use gtm::shared::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction};
use gtm::shared::playlist::PlaylistFormatKind;
use gtm::shared::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use gtm::shared::track::{LrcData, LrcLine, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use gtm::shared::wire::{decode, encode};

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

/// Like [`roundtrip!`] but JSON-only. `DaemonState` uses `#[serde(flatten)]`
/// for its `audio` settings, which bincode (a non-self-describing format)
/// cannot round-trip; JSON preserves the flat wire schema.
macro_rules! roundtrip_json {
    ($name:ident, $ty:ty, $val:expr) => {
        #[test]
        fn $name() {
            let val: $ty = $val;
            let json = serde_json::to_string(&val).unwrap();
            let de: $ty = serde_json::from_str(&json).unwrap();
            assert_eq!(
                format!("{:?}", val),
                format!("{:?}", de),
                "JSON round-trip failed"
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
        words: vec![gtm::shared::track::LrcWord {
            time: 12.5,
            text: "hello".into(),
        }],
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
            words: Vec::new(),
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
roundtrip_json!(daemon_state_roundtrip, DaemonState, sample_state());
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
fn req_cmd_name() {
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
        DaemonReq::SetSpeed { rate: 1.5 },
        DaemonReq::GetSpeed,
        DaemonReq::SetLowPower { enabled: true },
        DaemonReq::GetLowPower,
        DaemonReq::ListAudioDevices,
        DaemonReq::SetAudioDevice {
            name: Some("Speakers".into()),
        },
        DaemonReq::ToggleShuffle,
        DaemonReq::ToggleMute,
        DaemonReq::SetMono { enabled: true },
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
fn req_parse_unknown() {
    let result = DaemonReq::parse_cmd("totally_unknown", serde_json::json!({}));
    assert!(result.is_err());
}

#[test]
fn req_parse_play() {
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
fn req_parse_lastfm() {
    let params = serde_json::json!({
        "enabled": true,
        "api_key": "k1",
        "api_secret": "s1",
        "session_key": null,
        "min_play_secs": null,
        "min_play_pct": 0.5
    });
    let req = DaemonReq::parse_cmd("lastfm_set_config", params).unwrap();
    assert_eq!(req.cmd_name(), "lastfm_set_config");
    match req {
        DaemonReq::LastfmSetConfig {
            enabled,
            api_key,
            api_secret,
            session_key,
            min_play_secs,
            min_play_pct,
        } => {
            assert!(enabled);
            assert_eq!(api_key.as_deref(), Some("k1"));
            assert_eq!(api_secret.as_deref(), Some("s1"));
            assert!(session_key.is_none());
            assert!(min_play_secs.is_none());
            assert_eq!(min_play_pct, Some(0.5));
        }
        other => panic!("expected LastfmSetConfig, got {other:?}"),
    }

    for cmd in ["lastfm_auth_url", "lastfm_status", "lastfm_clear"] {
        let req = DaemonReq::parse_cmd(cmd, serde_json::json!({})).unwrap();
        assert_eq!(req.cmd_name(), cmd);
    }

    let req =
        DaemonReq::parse_cmd("lastfm_authenticate", serde_json::json!({ "token": "t1" })).unwrap();
    match req {
        DaemonReq::LastfmAuthenticate { token } => assert_eq!(token, "t1"),
        other => panic!("expected LastfmAuthenticate, got {other:?}"),
    }
}

#[test]
fn req_parse_unit() {
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
fn req_parse_spotify() {
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
        (
            "spotify_resolve_track",
            serde_json::json!({
                "name": "Drift",
                "artists": "Artist",
                "album": "Album",
                "uri": "spotify:track:abc123"
            }),
            "spotify_resolve_track",
        ),
        (
            "spotify_resolve_track",
            serde_json::json!({ "name": "Drift", "artists": "Artist", "album": "Album" }),
            "spotify_resolve_track",
        ),
        (
            "spotify_play_all",
            serde_json::json!({ "playlist_id": "37i9dQZEVX", "shuffle": true }),
            "spotify_play_all",
        ),
        (
            "spotify_play_all",
            serde_json::json!({ "playlist_id": "37i9dQZEVX" }),
            "spotify_play_all",
        ),
        (
            "spotify_track_image",
            serde_json::json!({ "image_url": "https://i.scdn.co/image/abc" }),
            "spotify_track_image",
        ),
        (
            "spotify_play_pause",
            serde_json::json!({}),
            "spotify_play_pause",
        ),
    ];
    for (cmd, params, expected) in cases {
        let req = DaemonReq::parse_cmd(cmd, params.clone()).unwrap();
        assert_eq!(req.cmd_name(), expected);
        match req {
            DaemonReq::SpotifySetToken { token } => assert_eq!(token, "BQCabc"),
            DaemonReq::SpotifyPlaylistTracks { id } => assert_eq!(id, "37i9dQZEVX"),
            DaemonReq::SpotifyResolve {
                playlist_id,
                track_index,
                ..
            } => {
                assert_eq!(playlist_id, "37i9dQZEVX");
                assert_eq!(track_index, 3);
            }
            DaemonReq::SpotifyResolveTrack { name, uri, .. } => {
                assert_eq!(name, "Drift");
                if params.get("uri").is_some() {
                    assert_eq!(uri.as_deref(), Some("spotify:track:abc123"));
                } else {
                    assert!(uri.is_none(), "uri must default to None when absent");
                }
            }
            DaemonReq::SpotifyPlayAll {
                playlist_id,
                shuffle,
            } => {
                assert_eq!(playlist_id, "37i9dQZEVX");
                if params.get("shuffle").is_some() {
                    assert!(shuffle);
                } else {
                    assert!(!shuffle, "shuffle must default to false when absent");
                }
            }
            DaemonReq::SpotifyTrackImage { image_url } => {
                assert_eq!(image_url, "https://i.scdn.co/image/abc");
            }
            _ => {}
        }
    }
}

#[test]
fn res_spotify_wire() {
    let status = SpotifyStatus {
        linked: true,
        user: Some("test-user".into()),
        premium: true,
        playing: false,
        device: Some("Test Speaker".into()),
        device_id: Some("dev-1".into()),
        shuffle: false,
        repeat: "off".into(),
        playlists: 2,
        tracks: 5,
        needs_relink: false,
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
            image_url: None,
            kind: None,
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
            "spotify_play_pause",
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
        (
            "spotify_track_image",
            DaemonRes::SpotifyImageRes {
                data: Some("AAECAw==".into()),
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

#[test]
fn res_lastfm_wire() {
    let cases: Vec<(&str, DaemonRes)> = vec![
        (
            "lastfm_auth_url",
            DaemonRes::LastfmAuthUrlRes {
                url: "https://www.last.fm/api/auth/?api_key=k1".into(),
            },
        ),
        (
            "lastfm_status",
            DaemonRes::LastfmStatusRes {
                enabled: true,
                api_key: Some("k1".into()),
                session_token: Some("sess".into()),
                ready: true,
                loved: false,
                error: None,
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

#[test]
fn res_oauth_wire() {
    for res in [
        DaemonRes::SpotifyOauthStarted {
            url: "https://accounts.spotify.com/authorize?response_type=code&client_id=c1".into(),
        },
        DaemonRes::SpotifyOauthStarted {
            url: "https://accounts.spotify.com/authorize?client_id=c2&scope=playlist-read-private"
                .into(),
        },
    ] {
        let expected = format!("{:?}", res);
        let wire = res.to_wire(1);
        let back = DaemonRes::from_wire("spotify_oauth_start", &wire);
        assert_eq!(expected, format!("{:?}", back));
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
    event_play_started,
    DaemonEvent::PlaybackStarted {
        track: sample_track(),
        auto_advanced: false,
        time_pos: 0.0,
        duration: 240.0,
    }
);
wire_event_roundtrip!(
    event_play_paused,
    DaemonEvent::PlaybackPaused { time_pos: 0.0 }
);
wire_event_roundtrip!(event_track_ended, DaemonEvent::TrackEnded);
wire_event_roundtrip!(event_volume, DaemonEvent::VolumeChanged { volume: 50 });
wire_event_roundtrip!(
    event_low_power,
    DaemonEvent::LowPowerChanged { enabled: true }
);
wire_event_roundtrip!(
    event_device_changed,
    DaemonEvent::AudioDeviceChanged {
        name: Some("Speakers".into())
    }
);

// ---------------------------------------------------------------------------
// IPC: DaemonRes serde round-trips (internally tagged, JSON-only)
// ---------------------------------------------------------------------------

#[test]
fn res_json_roundtrip() {
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
fn queue_action_json() {
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
fn lib_action_json() {
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
        LibraryAction::ImportPlaylist {
            path: "/m.m3u8".into(),
            format: PlaylistFormatKind::M3u8,
        },
        LibraryAction::ExportPlaylist {
            playlist_id: 1,
            path: "/out.pls".into(),
            format: PlaylistFormatKind::Pls,
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
        LibraryAction::PlaylistDedup { playlist_id: 1 },
        LibraryAction::PlaylistDoctor { playlist_id: 1 },
        LibraryAction::PlaylistSort {
            playlist_id: 1,
            field: "artist".into(),
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
fn decode_partial_none() {
    let events = vec![DaemonEvent::PlaybackPaused { time_pos: 0.0 }];
    let buf = encode(&events).unwrap();
    // Truncate to only length prefix
    assert!(decode(&buf[..2]).unwrap().is_none());
    // Truncate to just past length prefix
    assert!(decode(&buf[..5]).unwrap().is_none());
}

#[test]
fn decode_trunc_none() {
    let corrupted = vec![0u8, 0, 0, 5, 0xff, 0xff, 0xff];
    assert!(decode(&corrupted).unwrap().is_none());
}

#[test]
fn decode_corrupt_err() {
    // Length says 4 bytes, but content is not valid bincode
    let bad = b"\x00\x00\x00\x04\xff\xff\xff\xff".to_vec();
    assert!(decode(&bad).is_err());
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

#[test]
fn trans_stop_play() {
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
fn trans_seek_clamp() {
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
fn trans_volume_clamp() {
    let mut s = sample_state();
    s.set_volume(200).unwrap();
    assert_eq!(s.volume, 100);
}

#[test]
fn trans_shuffle() {
    let mut s = sample_state();
    assert!(!s.shuffle);
    s.toggle_shuffle().unwrap();
    assert!(s.shuffle);
    s.toggle_shuffle().unwrap();
    assert!(!s.shuffle);
}

#[test]
fn trans_repeat_cycle() {
    let mut s = sample_state();
    s.set_repeat_mode(RepeatMode::One).unwrap();
    assert_eq!(s.repeat, RepeatMode::One);
    s.set_repeat_mode(RepeatMode::All).unwrap();
    assert_eq!(s.repeat, RepeatMode::All);
}

#[test]
fn trans_mute_toggle() {
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
fn trans_crossfade() {
    let mut s = sample_state();
    s.set_crossfade(true, 99).unwrap();
    assert_eq!(s.crossfade.as_ref().unwrap().duration_secs, 30);
}

#[test]
fn trans_advance_once() {
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
fn trans_advance_empty() {
    let mut s = DaemonState::new();
    assert!(s.advance_queue().unwrap().is_none());
}

#[test]
fn trans_version() {
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
fn repeat_mode_changed() {
    let mut s = sample_state();
    s.apply_event(&DaemonEvent::RepeatModeChanged {
        mode: RepeatMode::One,
    });
    assert_eq!(s.repeat, RepeatMode::One);
}

#[test]
fn event_increments_version() {
    let mut s = sample_state();
    let v0 = s.version;
    s.apply_event(&DaemonEvent::PlaybackPaused { time_pos: 0.0 });
    assert_eq!(s.version, v0 + 1);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn crossfade_clamps() {
    let c = CrossfadeConfig::new(true, 40);
    assert_eq!(c.duration_secs, 30);
    let c = CrossfadeConfig::new(true, 5);
    assert_eq!(c.duration_secs, 5);
}

#[test]
fn state_defaults() {
    let s = DaemonState::new();
    assert_eq!(s.status, PlaybackStatus::Stopped);
    assert_eq!(s.volume, 100);
    assert_eq!(s.version, 0);
    assert!(s.queue.is_empty());
    assert_eq!(s.queue_cursor, 0);
    assert!(s.current_track.is_none());
}

#[test]
fn track_info_ok() {
    let t = sample_track();
    assert!(t.is_valid());
}

#[test]
fn track_no_path() {
    let mut t = sample_track();
    t.path.clear();
    assert!(!t.is_valid());
}

#[test]
fn track_no_hash() {
    let mut t = sample_track();
    t.hash.clear();
    assert!(!t.is_valid());
}

#[test]
fn track_neg_duration() {
    let mut t = sample_track();
    t.duration = -1.0;
    assert!(!t.is_valid());
}

#[test]
fn track_duration_fmt() {
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
fn malformed_json_err() {
    let result: Result<TrackInfo> = serde_json::from_str("not valid json").map_err(Into::into);
    assert!(result.is_err());
}

#[test]
fn trunc_bincode_err() {
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
fn unknown_cmd_err() {
    let result = DaemonReq::parse_cmd("unknown_command", serde_json::json!({}));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

#[test]
fn inv_passes_valid() {
    let s = sample_state();
    // Should not panic
    s.check_invariants();
}

#[test]
#[should_panic(expected = "volume 101 exceeds 100")]
fn inv_volume_high() {
    let mut s = sample_state();
    s.volume = 101;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "out of bounds")]
fn inv_cursor_oob() {
    let mut s = sample_state();
    s.queue_cursor = 99;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "negative time_pos")]
fn inv_neg_time() {
    let mut s = sample_state();
    s.time_pos = -1.0;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "Playing but current_track is None")]
fn inv_no_track() {
    let mut s = sample_state();
    s.status = PlaybackStatus::Playing;
    s.current_track = None;
    s.check_invariants();
}

#[test]
#[should_panic(expected = "crossfade enabled with duration_secs = 0")]
fn inv_crossfade_zero() {
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
fn state_defaults_eq() {
    let a = DaemonState::new();
    let b = DaemonState::default();
    assert_eq!(a.version, b.version);
    assert_eq!(a.status, b.status);
    assert_eq!(a.volume, b.volume);
}

// ---------------------------------------------------------------------------
// Request round-trip
// ---------------------------------------------------------------------------

/// Every source the request shapes are declared in, so the helpers below can
/// read type definitions instead of hand-maintaining a fixture per command.
const REQUEST_SOURCES: &str = concat!(
    include_str!("../src/shared/ipc.rs"),
    "\n",
    include_str!("../src/shared/state.rs"),
);

/// Bodies of every `struct <name> { ... }` in [`REQUEST_SOURCES`], brace
/// matched so nested braces inside a field type do not end the search early.
fn struct_bodies<'a>(src: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = src[cursor..].find(name) {
        let at = cursor + rel;
        cursor = at + name.len();
        // The name must be a declaration, not a use of the type.
        if !src[..at].ends_with("struct ") {
            continue;
        }
        let Some(rel) = src[cursor..].find('{') else {
            break;
        };
        let open = cursor + rel;
        let mut depth = 0usize;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(&src[open + 1..open + i]);
                        cursor = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            break;
        }
    }
    out
}

/// `name -> type` for the fields of one `struct` body.
fn fields_of(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, ty)) = line.split_once(':') else {
            continue;
        };
        let (name, ty) = (name.trim(), ty.trim().trim_end_matches(','));
        // `#[serde(default)] fn ...` lines and stray attributes slip
        // through the naive split; a field name is always lowercase.
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && !ty.is_empty();
        if valid {
            out.push((name.to_string(), ty.to_string()));
        }
    }
    out
}

/// One `parse_cmd` arm: the wire name it answers to, and the fields its
/// `struct Params` declares. Unit-variant arms declare no struct and take no
/// parameters.
struct Arm {
    name: String,
    fields: Vec<(String, String)>,
}

/// Every `"<wire name>" =>` arm in `parse_cmd`, in source order.
///
/// The arms are read individually rather than merged into one field map,
/// because the same field name means different things in different arms
/// (`action` is a `QueueAction` in one and a `LibraryAction` in another), so a
/// shared map would hand an arm a value of the wrong type.
fn request_arms() -> Vec<Arm> {
    let src = include_str!("../src/shared/ipc.rs");
    let start = src.find("pub fn parse_cmd").expect("parse_cmd in ipc.rs");
    // Bound the body to the function itself. Running to end-of-file would also
    // pick up the `yt_download_*` match further down, which is not a request
    // arm at all.
    let open = src[start..]
        .find('{')
        .map(|rel| start + rel)
        .expect("parse_cmd body");
    let mut nesting = 0usize;
    let end = src[open..]
        .char_indices()
        .find_map(|(i, c)| match c {
            '{' => {
                nesting += 1;
                None
            }
            '}' => {
                nesting -= 1;
                (nesting == 0).then_some(open + i)
            }
            _ => None,
        })
        .expect("parse_cmd is brace balanced");
    let body = &src[start..end];

    // A match arm starts on its own line, indented 12 spaces, with the wire
    // name in double quotes followed by `=>`. Track byte offsets in one pass.
    let mut arms: Vec<(usize, String)> = Vec::new();
    let mut off = 0usize;
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("            \"")
            && rest.contains("\" =>")
            && let Some(name) = rest.split('"').next()
            && !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            arms.push((off, name.to_string()));
        }
        off += line.len() + 1;
    }

    let mut out = Vec::new();
    for (i, (off, name)) in arms.iter().enumerate() {
        let end = arms.get(i + 1).map_or(body.len(), |(next, _)| *next);
        let arm = &body[*off..end];
        let fields = struct_bodies(arm, "Params")
            .into_iter()
            .flat_map(fields_of)
            .collect();
        out.push(Arm {
            name: name.clone(),
            fields,
        });
    }
    out
}

/// A JSON value that deserializes into `ty`. Scalars come straight from the
/// spelling of the type; an enum is resolved to its first variant so the
/// request decodes instead of failing on an unknown tag.
fn sample_for(ty: &str, depth: usize) -> serde_json::Value {
    use serde_json::Value;
    let ty = ty.trim();
    // Peel `Option` and `Vec` wrappers down to what they contain.
    if let Some(inner) = ty.strip_prefix("Option<").and_then(|r| r.strip_suffix('>')) {
        return if depth == 0 {
            Value::Null
        } else {
            sample_for(inner, depth - 1)
        };
    }
    if let Some(inner) = ty.strip_prefix("Vec<").and_then(|r| r.strip_suffix('>')) {
        return Value::Array(if depth == 0 {
            vec![]
        } else {
            vec![sample_for(inner, depth - 1)]
        });
    }
    match ty {
        "String" | "&str" | "PathBuf" => return Value::String(String::new()),
        "bool" => return Value::Bool(false),
        "f32" | "f64" => return Value::from(0.0),
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" => return Value::from(0),
        _ => {}
    }
    if depth == 0 {
        return Value::Null;
    }
    // An enum: take its first variant. A unit variant decodes from a string; a
    // struct variant from an externally tagged object.
    let Some(body) = enum_variants(REQUEST_SOURCES, ty) else {
        return Value::Null;
    };
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"));
    let Some(first) = first else {
        return Value::Null;
    };
    if !first.contains('{') {
        let variant = first.trim_end_matches(',');
        return Value::String(variant.to_string());
    }
    // The variant's fields continue on the lines below, so brace match from
    // the opening brace instead of reading the declaration line alone. The
    // counter is `nesting`, not `depth`: `depth` is the recursion budget.
    let start = body.find(first).unwrap_or_default();
    let Some(open) = body[start..].find('{').map(|rel| start + rel) else {
        return Value::Null;
    };
    let mut nesting = 0usize;
    let mut inner = "";
    for (i, c) in body[open..].char_indices() {
        match c {
            '{' => nesting += 1,
            '}' => {
                nesting -= 1;
                if nesting == 0 {
                    inner = &body[open + 1..open + i];
                    break;
                }
            }
            _ => {}
        }
    }
    let mut fields = serde_json::Map::new();
    for line in inner.lines() {
        let line = line.trim();
        let Some((n, t)) = line.split_once(':') else {
            continue;
        };
        let n = n.trim();
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        fields.insert(
            n.to_string(),
            sample_for(t.trim().trim_end_matches(','), depth - 1),
        );
    }
    let variant = first.split('{').next().unwrap_or_default().trim();
    let mut tagged = serde_json::Map::new();
    tagged.insert(variant.to_string(), Value::Object(fields));
    Value::Object(tagged)
}

/// Body of `enum <name> { ... }` in [`REQUEST_SOURCES`], brace matched.
fn enum_variants<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = src.find(&format!("enum {name}"))?;
    let open = src[at..].find('{')? + at;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open + 1..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every wire name `parse_cmd` accepts, read straight out of the source so a
/// newly added variant is covered without touching this test.
fn wire_names() -> Vec<&'static str> {
    include_str!("../src/shared/ipc.rs")
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.starts_with("DaemonReq::")
                .then(|| l.split('"').nth(1))
                .flatten()
        })
        .collect()
}

#[test]
fn req_every_wire_name_is_reachable() {
    let names = wire_names();
    assert!(names.len() >= 100, "only found {} wire names", names.len());
    let mut sorted = names.clone();
    sorted.sort_unstable();
    let total = sorted.len();
    sorted.dedup();
    assert_eq!(sorted.len(), total, "duplicate wire name in cmd_name");
}

#[test]
fn req_round_trip() {
    // Each arm gets a value of its own declared type for every field, so the
    // name must produce *a* request rather than an error. A name that falls
    // through to the catch-all, or whose field list drifted, fails here.
    for arm in request_arms() {
        let params: serde_json::Value = arm
            .fields
            .iter()
            .map(|(n, ty)| (n.clone(), sample_for(ty, 4)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        let req = DaemonReq::parse_cmd(&arm.name, params)
            .unwrap_or_else(|e| panic!("{} failed to decode: {e}", arm.name));
        assert_eq!(
            req.cmd_name(),
            arm.name,
            "{} decoded to a different command",
            arm.name
        );
    }
}

#[test]
fn req_arms_were_read() {
    // Guards the source scraping. If the arm shape ever moves, the round-trip
    // would silently weaken to decoding nothing.
    let arms = request_arms();
    assert!(arms.len() >= 100, "only found {} arms", arms.len());
    assert_eq!(arms.len(), wire_names().len(), "arm count drifted");
    let with_params = arms.iter().filter(|a| !a.fields.is_empty()).count();
    assert!(
        with_params >= 60,
        "only {with_params} arms declare parameters"
    );
    for arm in &arms {
        if arm.name == "play" {
            let names: Vec<&str> = arm.fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(names.contains(&"path"), "play lost its path field");
            assert!(names.contains(&"start_pos"), "play lost start_pos");
        }
    }
}

#[test]
fn req_every_variant_has_an_arm() {
    // `cmd_name` and `parse_cmd` are two independent listings of the wire
    // protocol. A variant with no arm decodes to the catch-all; an arm with no
    // variant is dead weight. Either means the two have drifted apart.
    let arms: Vec<String> = request_arms().into_iter().map(|a| a.name).collect();
    let names = wire_names();
    let mut missing: Vec<&str> = names
        .iter()
        .filter(|n| !arms.iter().any(|a| a == *n))
        .copied()
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "variants with no parse arm: {missing:?}"
    );

    let mut orphan: Vec<&str> = arms
        .iter()
        .map(String::as_str)
        .filter(|n| !names.contains(n))
        .collect();
    orphan.sort_unstable();
    assert!(orphan.is_empty(), "arms with no variant: {orphan:?}");
}

#[test]
fn req_unit_variant_ignores_params() {
    for name in wire_names() {
        if let Ok(req) = DaemonReq::parse_cmd(name, serde_json::json!({"bogus": 1})) {
            assert_eq!(req.cmd_name(), name);
        }
    }
}

// ---------------------------------------------------------------------------
// Spotify picker
// ---------------------------------------------------------------------------

/// `PickerSource` cycles over six values, but the Spotify search picker only
/// has rows for four of them. `spot_picks` maps `Radio` to "no rows", so a
/// filter of `Radio` must yield an empty list rather than every result.
#[test]
fn spot_source_cycles_are_all_reachable() {
    use gtm::picker::PickerSource;
    let mut s = PickerSource::default();
    let mut seen = vec![s];
    for _ in 0..8 {
        s = s.next();
        if seen.contains(&s) {
            break;
        }
        seen.push(s);
    }
    assert_eq!(seen.len(), 6, "source cycle has {} stops", seen.len());
    assert!(seen.contains(&PickerSource::Radio));
}

/// Cover URLs ride on the search row, so every web result must carry one that
/// the daemon can fetch; a row without a URL renders an empty preview.
#[test]
fn spot_track_carries_cover_url() {
    let t = SpotifyTrack {
        index: 0,
        name: "Song".into(),
        artists: "Artist".into(),
        album: Some("Album".into()),
        duration_ms: Some(210_000),
        uri: Some("spotify:track:abc".into()),
        image_url: Some("https://i.scdn.co/image/x".into()),
        kind: None,
    };
    // Round-trips over the wire, which is how the TUI receives it.
    let json = serde_json::to_value(&t).expect("track serialises");
    let back: SpotifyTrack = serde_json::from_value(json).expect("track deserialises");
    assert_eq!(back.image_url, t.image_url);
    assert_eq!(back.uri, t.uri);
    assert!(back.has_uri());
    // A track kind is omitted on the wire (it is the default), so a track
    // result must decode back to `None` and stay in the Tracks filter.
    assert_eq!(back.kind, None);
}

#[test]
fn spot_album_kind_survives_wire() {
    use gtm::shared::spotify::SpotifySearchKind;
    let t = SpotifyTrack {
        index: 0,
        name: "Album".into(),
        artists: String::new(),
        album: Some("Album".into()),
        duration_ms: None,
        uri: Some("spotify:album:abc".into()),
        image_url: None,
        kind: Some(SpotifySearchKind::Album),
    };
    let json = serde_json::to_value(&t).expect("track serialises");
    let back: SpotifyTrack = serde_json::from_value(json).expect("track deserialises");
    assert_eq!(back.kind, Some(SpotifySearchKind::Album));
}
