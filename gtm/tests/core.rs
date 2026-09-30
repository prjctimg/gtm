// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Integration tests for shared: serde, wire, state machine, and invariants
//
// This is free software released under the GPL-3.0 license.

use gtm::shared::Result;
use gtm::shared::chart::{ChartPlaylist, ChartSource, ChartTrack};
use gtm::shared::global::{
    CrossfadeConfig, DaemonState, Image, PlaybackStatus, RepeatMode, ThemeMode, UIMode, YTFilter,
};
use gtm::shared::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction};
use gtm::shared::playlist::PlaylistFormatKind;
use gtm::shared::podcast::PodcastFeed;
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

    for cmd in ["lastfm_status", "lastfm_clear"] {
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
        needs_play_link: false,
        error: None,
    };
    let playlist = SpotifyPlaylist {
        image_url: None,
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
    let cases: Vec<(&str, DaemonRes)> = vec![(
        "lastfm_status",
        DaemonRes::LastfmStatusRes {
            enabled: true,
            api_key: Some("k1".into()),
            session_token: Some("sess".into()),
            ready: true,
            loved: false,
            error: None,
        },
    )];
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

/// Every response variant the client matches on a named `cmd` must be
/// reconstructible by `from_wire` for that same `cmd`. A missing arm decodes to
/// a bare `DaemonRes::Value`, which the client then rejects as "unexpected
/// response" while the daemon had answered correctly — lyrics for any provider
/// track, Top Charts, podcast add-feed and Last.fm linking all failed this way.
///
/// Driven off the client side rather than a hand-written list, so a new typed
/// expectation cannot be added without the decode arm that serves it.
#[test]
fn every_client_expectation_round_trips() {
    let cases: Vec<(&str, DaemonRes)> = vec![
        (
            "get_lyrics",
            DaemonRes::Lyrics {
                lyrics: Some(sample_lrc()),
            },
        ),
        // The provider-track route: no library row, so searched by name. This is
        // the one that reported "unexpected response" for every Spotify row.
        (
            "lyrics_search",
            DaemonRes::Lyrics {
                lyrics: Some(sample_lrc()),
            },
        ),
        // A miss is `None`, not an absent key, and must survive as `None`.
        ("lyrics_search", DaemonRes::Lyrics { lyrics: None }),
        (
            "charts_sources",
            DaemonRes::ChartsSourcesRes {
                sources: vec![ChartSource {
                    id: "spotify".into(),
                    display: "Spotify Charts".into(),
                    configured: true,
                }],
            },
        ),
        (
            "charts_list",
            DaemonRes::ChartsListRes {
                charts: vec![ChartPlaylist {
                    source_id: "spotify".into(),
                    id: "37i9dQ".into(),
                    title: "Today's Top Hits".into(),
                    description: None,
                    cover_url: None,
                    owner: None,
                    track_count: Some(50),
                }],
            },
        ),
        (
            "charts_tracks",
            DaemonRes::ChartsTracksRes {
                tracks: vec![ChartTrack {
                    index: 0,
                    title: "Rhyme Dust".into(),
                    artists: "MK, Dom Dolla".into(),
                    album: Some("Rhyme Dust".into()),
                    duration_ms: Some(200_000),
                    uri: "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into(),
                    cover_url: None,
                }],
            },
        ),
        (
            "podcast_add_feed",
            DaemonRes::PodcastFeedsRes {
                feeds: vec![sample_feed()],
            },
        ),
        // With a `feed_id` the daemon answers feeds; with none it answers
        // `Value { refreshed }`, which the client's own `Value` arm reads. Both
        // shapes have to survive the same `cmd`.
        (
            "podcast_refresh",
            DaemonRes::PodcastFeedsRes {
                feeds: vec![sample_feed()],
            },
        ),
        (
            "podcast_refresh",
            DaemonRes::Value {
                value: serde_json::json!({ "refreshed": 3 }),
            },
        ),
        (
            "lastfm_oauth_start",
            DaemonRes::LastfmAuthUrlRes {
                url: "https://www.last.fm/api/auth/?api_key=k1&token=t1".into(),
            },
        ),
    ];
    for (cmd, res) in cases {
        let expected = format!("{res:?}");
        let back = DaemonRes::from_wire(cmd, &res.to_wire(1));
        assert_eq!(
            expected,
            format!("{back:?}"),
            "{cmd} did not round-trip into the variant the client matches on"
        );
    }
}

fn sample_feed() -> PodcastFeed {
    PodcastFeed {
        id: "feed-1".into(),
        title: "A Feed".into(),
        url: "https://example.com/feed.xml".into(),
        description: String::new(),
        episodes: 12,
    }
}

fn sample_lrc() -> LrcData {
    LrcData {
        title: Some("Rhyme Dust".into()),
        artist: Some("MK, Dom Dolla".into()),
        album: Some("Rhyme Dust".into()),
        lines: vec![
            LrcLine {
                timestamp: 0.06,
                text: "Right here".into(),
                words: Vec::new(),
            },
            LrcLine {
                timestamp: 14.57,
                text: "Rhyme dust".into(),
                words: Vec::new(),
            },
        ],
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

/// The fields of one `struct` body, noting which carry `#[serde(flatten)]`.
fn fields_of(body: &str) -> Vec<ParamField> {
    let mut out = Vec::new();
    // An attribute on its own line applies to the field that follows it.
    let mut flatten = false;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(attr) = line.strip_prefix("#[") {
            flatten = attr.contains("flatten");
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
            out.push(ParamField {
                name: name.to_string(),
                ty: ty.to_string(),
                flatten,
            });
        }
        flatten = false;
    }
    out
}

/// One `parse_cmd` arm: the wire name it answers to, and the fields its
/// `struct Params` declares. Unit-variant arms declare no struct and take no
/// parameters.
struct Arm {
    name: String,
    fields: Vec<ParamField>,
}

/// One field of a `Params` struct. `flatten` matters: a flattened field's
/// value is spread across the params object rather than nested under its own
/// name, which is exactly how `queue` and `library` carry their action.
struct ParamField {
    name: String,
    ty: String,
    flatten: bool,
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
    // An enum: take its first variant, spelled the way serde spells it. A unit
    // variant decodes from a bare string; a struct variant from an object that
    // is either externally tagged (`{"scan": {...}}`) or internally tagged
    // (`{"action": "scan", ...}`) depending on the enum's attributes.
    let Some(meta) = enum_meta(REQUEST_SOURCES, ty) else {
        return Value::Null;
    };
    let first = meta
        .body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"));
    let Some(first) = first else {
        return Value::Null;
    };
    // `first` is a whole line of the enum body, so it still carries the comma
    // that separates it from the next variant.
    let variant = rename_variant(
        first
            .split('{')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_end_matches(',')
            .trim(),
        meta.rename_all,
    );
    if !first.contains('{') && meta.tag.is_none() {
        return Value::String(variant);
    }
    // The variant's fields continue on the lines below, so brace match from
    // the opening brace instead of reading the declaration line alone. The
    // counter is `nesting`, not `depth`: `depth` is the recursion budget.
    let mut fields = serde_json::Map::new();
    if first.contains('{') {
        let start = meta.body.find(first).unwrap_or_default();
        let Some(open) = meta.body[start..].find('{').map(|rel| start + rel) else {
            return Value::Null;
        };
        let mut nesting = 0usize;
        let mut inner = "";
        for (i, c) in meta.body[open..].char_indices() {
            match c {
                '{' => nesting += 1,
                '}' => {
                    nesting -= 1;
                    if nesting == 0 {
                        inner = &meta.body[open + 1..open + i];
                        break;
                    }
                }
                _ => {}
            }
        }
        for f in fields_of(inner) {
            fields.insert(f.name, sample_for(&f.ty, depth - 1));
        }
    }
    let mut out = serde_json::Map::new();
    match meta.tag {
        // Internally tagged: the tag wraps unit variants too, so `List` is
        // `{"action": "list"}` and not the bare string `"list"`.
        Some(tag) => {
            out.insert(tag.to_string(), Value::String(variant));
            out.extend(fields);
        }
        None => {
            out.insert(variant, Value::Object(fields));
        }
    }
    Value::Object(out)
}

/// How an enum is spelled on the wire, read from the attributes above it.
struct EnumMeta<'a> {
    body: &'a str,
    /// `rename_all` casing, if the enum sets one.
    rename_all: Option<&'a str>,
    /// `tag` key, if the enum is internally tagged.
    tag: Option<&'a str>,
}

/// Locate `enum <name>` in `src` and read the body plus the serde attributes
/// declared immediately above it.
fn enum_meta<'a>(src: &'a str, name: &str) -> Option<EnumMeta<'a>> {
    let decl = src.find(&format!("enum {name}"))?;
    // Start at the beginning of the declaration line: `pub enum Foo` puts a
    // `pub ` between the newline and `enum`, so walking back from `decl` would
    // stop on that instead of on the attributes.
    let decl_line = src[..decl].rfind('\n').map_or(0, |i| i + 1);
    // Walk back over the contiguous attribute block. A `#[serde(...)]` may be
    // one line or several, so accept a line that opens or continues one.
    let mut block_start = decl_line;
    while block_start > 0 {
        let head_end = block_start - 1;
        let Some(nl) = src[..head_end].rfind('\n') else {
            break;
        };
        let line = src[nl + 1..head_end].trim();
        let continues = line.starts_with("#[")
            || line.starts_with("rename_all")
            || line.starts_with("tag")
            || line.starts_with("untagged");
        if !continues {
            break;
        }
        block_start = nl + 1;
    }
    let attrs = &src[block_start..decl_line];
    Some(EnumMeta {
        body: enum_variants(src, name)?,
        rename_all: attr_value(attrs, "rename_all"),
        tag: attr_value(attrs, "tag"),
    })
}

/// Read `key = "value"` out of an attribute block.
fn attr_value<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key} = \"");
    let at = attrs.find(&pat)?;
    let rest = &attrs[at + pat.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Apply a `rename_all` casing to a Rust variant name, matching what serde
/// puts on the wire.
fn rename_variant(variant: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("snake_case") => {
            let mut out = String::with_capacity(variant.len() + 4);
            for (i, c) in variant.chars().enumerate() {
                if c.is_ascii_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            }
            out
        }
        Some("lowercase") => variant.to_ascii_lowercase(),
        Some("kebab-case") => variant.replace('_', "-").to_ascii_lowercase(),
        _ => variant.to_string(),
    }
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
        let params = arm_params(&arm);
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

/// Build the params object for one arm. A `#[serde(flatten)]` field is merged
/// into the object rather than nested under its name, so `queue` gets
/// `{"action":"add","paths":[]}` and not `{"action":{"action":"add",...}}`.
fn arm_params(arm: &Arm) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for field in &arm.fields {
        let value = sample_for(&field.ty, 4);
        match (field.flatten, value) {
            (true, serde_json::Value::Object(inner)) => map.extend(inner),
            (_, value) => {
                map.insert(field.name.clone(), value);
            }
        }
    }
    map.into()
}

#[test]
fn req_tagged_actions_survive_the_real_serializer() {
    // `req_round_trip` builds params by hand from the `Params` struct, so it
    // agrees with the decoder by construction and cannot catch the two
    // disagreeing. This goes through the actual serializer: `DaemonReq` is
    // `#[serde(untagged)]` and flattens its action, so the wire form has the
    // tag and the variant's fields at the top level — and `parse_cmd` has to
    // read them from there.
    let queue = serde_json::to_value(DaemonReq::Queue {
        action: QueueAction::Add {
            paths: vec!["/tmp/a.opus".into()],
            position: None,
        },
    })
    .expect("queue serialises");
    // `Option` fields have no `skip_serializing_if`, so the wire form spells
    // an absent `position` out as null.
    assert_eq!(
        queue,
        serde_json::json!({"action": "add", "paths": ["/tmp/a.opus"], "position": null})
    );
    assert!(matches!(
        DaemonReq::parse_cmd("queue", queue).expect("queue decodes"),
        DaemonReq::Queue {
            action: QueueAction::Add { .. }
        }
    ));

    // A unit variant flattens to just the tag.
    let list = serde_json::to_value(DaemonReq::Queue {
        action: QueueAction::List,
    })
    .expect("queue serialises");
    assert_eq!(list, serde_json::json!({"action": "list"}));
    assert!(matches!(
        DaemonReq::parse_cmd("queue", list).expect("queue decodes"),
        DaemonReq::Queue {
            action: QueueAction::List
        }
    ));

    let library = serde_json::to_value(DaemonReq::Library {
        action: LibraryAction::Scan {
            path: "/music".into(),
        },
    })
    .expect("library serialises");
    assert_eq!(
        library,
        serde_json::json!({"action": "scan", "path": "/music"})
    );
    assert!(matches!(
        DaemonReq::parse_cmd("library", library).expect("library decodes"),
        DaemonReq::Library {
            action: LibraryAction::Scan { .. }
        }
    ));
}

#[test]
fn req_enum_samples_match_their_serde_spelling() {
    // The whole point of `sample_for` is to produce a value the decoder
    // accepts, so the three ways an enum spells itself on the wire each need a
    // case: plain, `rename_all`, and internally tagged.
    assert_eq!(sample_for("RepeatMode", 4), serde_json::json!("Off"));
    assert_eq!(sample_for("EqPreset", 4), serde_json::json!("flat"));
    assert_eq!(sample_for("CacheKind", 4), serde_json::json!("lyrics"));
    // `#[serde(tag = "action", rename_all = "snake_case")]`: the variant's
    // fields sit beside the tag, not nested under it — and the tag wraps unit
    // variants too, so `List` is `{"action": "list"}` and not `"list"`.
    assert_eq!(
        sample_for("LibraryAction", 4),
        serde_json::json!({"action": "scan", "path": ""})
    );
    assert_eq!(
        sample_for("QueueAction", 4),
        serde_json::json!({"action": "list"})
    );
    // Wrappers peel before the enum is resolved.
    assert_eq!(sample_for("Option<EqPreset>", 4), serde_json::json!("flat"));
    assert_eq!(sample_for("Option<EqPreset>", 0), serde_json::Value::Null);
    assert_eq!(sample_for("Vec<EqPreset>", 4), serde_json::json!(["flat"]));
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
            let names: Vec<&str> = arm.fields.iter().map(|f| f.name.as_str()).collect();
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

// ---------------------------------------------------------------------------
// Settings rows
// ---------------------------------------------------------------------------

/// `(category, row labels)` read out of the one declaration the Settings pane is
/// built from, `ui/pickers/settings_rows.rs`.
///
/// The row list, the rendered labels, the Enter handler and the navigation
/// bound used to be four independent `match`es over row numbers, and they
/// drifted: the Spotify transport rows were addressed by an index the handler
/// computed one way and the callee matched another, so Next, Previous and
/// Shuffle all ran `set_repeat`. Reading the declaration is what lets the tests
/// below check the other three against it instead of against each other.
fn settings_declared_rows() -> Vec<(u8, Vec<String>)> {
    let src = include_str!("../src/ui/pickers/settings_rows.rs");
    let mut out = Vec::new();
    for (cat, const_name) in [
        (0u8, "PLAYBACK_ROWS"),
        (1, "SYSTEM_ROWS"),
        (2, "SPOTIFY_ROWS"),
    ] {
        let needle = format!("pub(crate) const {const_name}: SettingsRows = &[");
        let start = src.find(&needle).expect(const_name);
        let body = &src[start + needle.len()..];
        let Some(close) = body.find("];") else {
            panic!("{const_name} has no closing bracket");
        };
        let labels: Vec<String> = body[..close]
            .lines()
            .filter_map(|l| l.trim().strip_prefix('('))
            .filter_map(|l| l.split_once('"'))
            .filter_map(|(_, rest)| rest.rsplit_once('"').map(|(s, _)| s.to_string()))
            .collect();
        assert!(
            !labels.is_empty(),
            "{const_name} parsed as zero rows:\n{}",
            &body[..close]
        );
        out.push((cat, labels));
    }
    out
}

/// The number of values `settings_values` produces per category, counted from
/// its `vec![...]` literals.
fn settings_value_counts() -> Vec<(u8, usize)> {
    let src = include_str!("../src/ui/pickers/settings.rs");
    let start = src.find("fn settings_values").expect("settings_values");
    let body = &src[start..];
    let mut out = Vec::new();
    for cat in 0u8..3 {
        let needle = format!("\n            {cat} => ");
        let Some(rel) = body.find(&needle) else {
            panic!("settings_values has no arm {cat}");
        };
        let arm = &body[rel..];
        let end = arm[1..]
            .find("\n            _ =>")
            .map(|i| i + 1)
            .unwrap_or(arm.len());
        let arm = &arm[..end];
        let Some(open) = arm.find("vec![") else {
            panic!("settings_values arm {cat} has no vec!");
        };
        let inner = &arm[open + "vec![".len()..];
        let Some(close) = inner.find("]") else {
            panic!("settings_values arm {cat} vec! is unterminated");
        };
        let mut nesting = 0usize;
        let mut rows = 0usize;
        let mut saw = false;
        for c in inner[..close].chars() {
            match c {
                '[' | '{' | '(' => nesting += 1,
                ']' | '}' | ')' => nesting -= 1,
                ',' if nesting == 0 => {
                    if saw {
                        rows += 1;
                    }
                    saw = false;
                }
                c if !c.is_whitespace() => saw = true,
                _ => {}
            }
        }
        if saw {
            rows += 1;
        }
        out.push((cat, rows));
    }
    out
}

/// `(category, covered row indices)` from the `N => match opt {` blocks in the
/// Settings Enter handler.
fn settings_arm_coverage() -> Vec<(u8, Vec<usize>)> {
    let src = include_str!("../src/app/settings_keys.rs");
    let mut out = Vec::new();
    for cat in 0u8..3 {
        let needle = format!("{cat} => match opt {{");
        let Some(start) = src.find(&needle) else {
            continue;
        };
        let body = &src[start..];
        let Some(open) = body.find('{') else { continue };
        let mut nesting = 0usize;
        let mut close = None;
        for (i, c) in body[open..].char_indices() {
            match c {
                '{' => nesting += 1,
                '}' => {
                    nesting -= 1;
                    if nesting == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { continue };
        let block = &body[open..close];
        let mut covered: Vec<usize> = Vec::new();
        for line in block.lines() {
            let trimmed = line.trim();
            // Only top-level arms: a nested match indents further than the
            // block's own opening line.
            let indent = line.len() - trimmed.len();
            if indent == 0 {
                continue;
            }
            let Some((pat, _)) = trimmed.split_once(" =>") else {
                continue;
            };
            if let Some((a, b)) = pat.split_once("..=") {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    covered.extend(a..=b);
                }
            } else {
                for part in pat.split('|') {
                    if let Ok(n) = part.trim().parse::<usize>() {
                        covered.push(n);
                    }
                }
            }
        }
        covered.sort_unstable();
        covered.dedup();
        out.push((cat, covered));
    }
    out
}

#[test]
fn settings_value_column_is_aligned_with_the_declared_rows() {
    // The label column comes from the declaration and the value column from a
    // `vec![...]` in the renderer, one entry per row, positionally. A missing
    // or extra entry does not fail to compile: it shifts every value below it
    // onto the wrong row, so the EQ toggle reads "On" while the reverb toggle
    // reads the cover provider.
    let declared: std::collections::BTreeMap<u8, usize> = settings_declared_rows()
        .into_iter()
        .map(|(c, l)| (c, l.len()))
        .collect();
    for (cat, values) in settings_value_counts() {
        let want = declared
            .get(&cat)
            .copied()
            .unwrap_or_else(|| panic!("category {cat} has no declaration"));
        assert_eq!(
            values, want,
            "category {cat} produces {values} values for {want} declared rows"
        );
    }
}

#[test]
fn settings_enter_arms_stay_inside_the_declared_rows() {
    // An arm past the end of the row list is unreachable at best, and a row
    // whose arm belongs to a different row is worse: it silently runs the wrong
    // action. Both happened when Spotify's handler was written against a
    // shorter row list than the pane rendered.
    let declared: std::collections::BTreeMap<u8, usize> = settings_declared_rows()
        .into_iter()
        .map(|(c, l)| (c, l.len()))
        .collect();
    for (cat, arms) in settings_arm_coverage() {
        let rows = declared
            .get(&cat)
            .copied()
            .unwrap_or_else(|| panic!("category {cat} has no declaration"));
        let beyond: Vec<usize> = arms.iter().copied().filter(|a| *a >= rows).collect();
        assert!(
            beyond.is_empty(),
            "category {cat} declares {rows} rows but has arms {beyond:?} past the end"
        );
    }
}

#[test]
fn settings_navigation_bound_comes_from_the_declaration() {
    // `category_options` is what clamps keyboard navigation. It used to be a
    // hand-written `match` of literals, a third thing to keep in step; it now
    // reads the declaration's own `len()`.
    let src = include_str!("../src/app/theme.rs");
    let fn_start = src.find("fn category_options").expect("category_options");
    let body = &src[fn_start..];
    let end = body.find("\n    }").unwrap_or(body.len());
    let body = &body[..end];
    assert!(
        body.contains("rows_for(") && body.contains(".len()"),
        "category_options must derive its count from rows_for(), not from literals:\n{body}"
    );
}

/// An untitled entry must never render as a raw provider URI.
///
/// The fallback was `Path::file_stem`, which on `spotify:track:4cOdK2w…`
/// returns the whole string, and on `spotify:track:abc.def` stops at the dot
/// and returns `spotify:track:abc`. It was written independently at eight
/// call sites and only one of them checked for a provider path, so a raw id
/// reached the queue rows, the now-playing widget, the search and library
/// pickers and the info card.
#[test]
fn display_title_never_leaks_a_uri() {
    let uri = TrackInfo {
        path: "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into(),
        ..Default::default()
    };
    let title = uri.display_title();
    assert!(!title.contains("spotify:"), "{title}");
    assert!(!title.contains(':'), "{title}");

    // The truncating case is the one a naive guard misses: the id itself
    // contains a dot, so a stem-based fallback silently returns a *valid
    // looking* but wrong id.
    let dotted = TrackInfo {
        path: "spotify:track:abc.def".into(),
        ..Default::default()
    };
    assert!(
        !dotted.display_title().contains("spotify:"),
        "{}",
        dotted.display_title()
    );
}

/// A real title always wins, and a real file still falls back to its stem.
#[test]
fn display_title_prefers_the_metadata() {
    let tagged = TrackInfo {
        path: "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into(),
        title: "Rhyme Dust".into(),
        ..Default::default()
    };
    assert_eq!(tagged.display_title(), "Rhyme Dust");

    let file = TrackInfo {
        path: "/music/some song.mp3".into(),
        ..Default::default()
    };
    assert_eq!(file.display_title(), "some song");
}

/// The library categories that are not backed by `filtered_tracks`.
///
/// A category with no arm there falls through to the whole local library, and
/// the left info card then renders `tracks_cache[list_pos()]` — so browsing
/// Top Charts described an unrelated local track. Charts are the case that
/// actually shipped broken; the rest are here so a future category is added to
/// the list rather than rediscovering the same failure.
#[test]
fn chart_rows_are_not_answered_from_the_local_library() {
    let src = include_str!("../src/app/cover.rs");
    let start = src
        .find("pub fn track_info_kind(&self) -> TrackInfoKind {")
        .expect("track_info_kind");
    let body = &src[start..];
    let end = body.find("\n    }").unwrap_or(body.len());
    let body = &body[..end];

    assert!(
        body.contains("12 => TrackInfoKind::ChartTrack"),
        "category 12 (Top Charts) must not fall through to TrackInfoKind::Track:\n{body}"
    );

    // And the other list-shaped categories, for the same reason.
    for (cat, kind) in [
        (2, "Album"),
        (3, "Artist"),
        (4, "Playlist"),
        (5, "SpotifyPlaylist"),
    ] {
        assert!(
            body.contains(&format!("{cat} => TrackInfoKind::{kind}")),
            "category {cat} must map to {kind}"
        );
    }
}

/// Every command-palette hint must have a dispatch arm, and vice versa.
///
/// A palette entry with no arm is a dead row; an arm with no entry is a feature
/// nobody can reach. The Spotify Connect controls were removed from the
/// Settings pane and re-homed here, and the two lists are the only place that
/// can catch a half-finished move.
#[test]
fn palette_hints_all_have_a_dispatch_arm() {
    let palette = include_str!("../src/ui/command.rs");
    let keys = include_str!("../src/app/keys.rs");

    let mut hints: Vec<String> = Vec::new();
    for line in palette.lines() {
        let Some(rest) = line.trim().strip_prefix("hint: \"") else {
            continue;
        };
        if let Some(h) = rest.strip_suffix("\",") {
            hints.push(h.to_string());
        }
    }
    assert!(hints.len() > 30, "only parsed {} hints", hints.len());

    let missing: Vec<&String> = hints
        .iter()
        .filter(|h| !keys.contains(&format!("action == \"{h}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "palette hints with no dispatch arm: {missing:?}"
    );

    // The four Connect controls must survive the move out of Settings.
    for h in [
        "spotify next",
        "spotify previous",
        "spotify shuffle",
        "spotify repeat",
    ] {
        assert!(hints.iter().any(|x| x == h), "{h} missing from the palette");
        assert!(
            keys.contains(&format!("action == \"{h}\"")),
            "{h} has no arm"
        );
    }
}

/// Spotify removed the endpoints the album/artist/playlist drill-downs used.
///
/// Spotify's February 2026 Web API changes removed `/albums`, `/artists` and
/// `/playlists/{id}/tracks` for developer-mode integrations. The three
/// drill-downs, their IPC variants and their client methods are gone; this
/// holds them gone and stops a future edit from wiring one back up, because the
/// failure mode is silent -- a dead drill-down closes the picker and does
/// nothing, which looks like a keybinding problem rather than a missing API.
#[test]
fn no_spotify_drill_down_over_removed_endpoints() {
    let files = [
        (
            "gtm/src/shared/ipc.rs",
            include_str!("../src/shared/ipc.rs"),
        ),
        (
            "gtm/src/shared/client.rs",
            include_str!("../src/shared/client.rs"),
        ),
        ("gtm/src/app/keys.rs", include_str!("../src/app/keys.rs")),
        (
            "gtmd/src/daemon/mod.rs",
            include_str!("../../gtmd/src/daemon/mod.rs"),
        ),
        (
            "gtmd/src/providers/spotify/cmd.rs",
            include_str!("../../gtmd/src/providers/spotify/cmd.rs"),
        ),
        (
            "gtmd/src/providers/spotify/api.rs",
            include_str!("../../gtmd/src/providers/spotify/api.rs"),
        ),
    ];

    for (name, src) in files {
        for gone in [
            "SpotifyAlbumTracks",
            "SpotifyArtistTopTracks",
            "SpotifyWebPlaylistTracks",
        ] {
            assert!(
                !src.contains(gone),
                "{name} still references {gone}, whose endpoint Spotify removed in 2026"
            );
        }
        // The api-level helpers, which are what actually issued the requests.
        for gone in [
            "pub async fn album_tracks",
            "pub async fn artist_top",
            "pub async fn web_playlist(",
        ] {
            assert!(!src.contains(gone), "{name} still defines {gone}");
        }
    }

    // The removal is announced in the UI, so a search hit that cannot be
    // opened says why rather than closing the picker on nothing.
    let keys = include_str!("../src/app/keys.rs");
    assert!(
        keys.contains("Spotify no longer exposes"),
        "no message for a search row that can no longer be opened"
    );
}

/// The library picker must resolve its rows through one filter everywhere.
///
/// The cursor bound, the renderer and the Enter handler each used to derive
/// "the list" independently: two read the configured lists, and once a search
/// box existed that made the highlighted row and the opened category different
/// things. `filtered_library_indices` is the single answer, and this is the
/// only place that can notice one of the three going back to reading the
/// unfiltered list.
#[test]
fn library_picker_rows_come_from_one_filter() {
    let app = include_str!("../src/app/mod.rs");
    let search = include_str!("../src/app/search.rs");
    let keys = include_str!("../src/app/keys.rs");
    let lib = include_str!("../src/ui/pickers/library.rs");

    assert!(
        search.contains("pub fn filtered_library_indices"),
        "the shared filter is gone"
    );

    // The two count sites in app/mod.rs.
    for arm in [
        "PickerId::Libraries => self.filtered_library_indices().len(),",
        "PickerId::Libraries => self.filtered_library_indices().len().saturating_sub(1),",
    ] {
        assert!(app.contains(arm), "count site bypasses the filter: {arm}");
    }

    // The renderer.
    assert!(
        lib.contains("let cats = app.filtered_library_indices();"),
        "the renderer bypasses the filter"
    );
    // The Enter handler.
    assert!(
        keys.contains("self.filtered_library_indices().get(sel).copied()"),
        "Enter bypasses the filter"
    );

    // Nothing in these four may read the unfiltered list to address a row.
    for (name, src) in [("app/mod.rs", app), ("ui/pickers/library.rs", lib)] {
        assert!(
            !src.contains("PickerId::Libraries => self.visible_library_indices().len()"),
            "{name} still bounds the cursor by the unfiltered list"
        );
    }
}

/// The library picker needs arrow-key navigation and a search box, and cannot
/// have both `j`/`k` navigation and a text query: they are the same keys.
#[test]
fn library_picker_has_arrows_and_search() {
    let keys = include_str!("../src/app/keys.rs");
    let lib = include_str!("../src/ui/pickers/library.rs");

    let start = keys
        .find("o.id == PickerId::Libraries")
        .expect("no nav block");
    let block = &keys[start..start + 1600];

    for want in [
        "KeyCode::Up =>",
        "KeyCode::Down =>",
        "KeyCode::Char(c)",
        "KeyCode::Backspace =>",
    ] {
        assert!(block.contains(want), "library picker missing {want}");
    }
    // `j`/`k` are query characters here, so they must not be navigation.
    assert!(
        !block.contains("Char('j')") && !block.contains("Char('k')"),
        "j/k would be eaten as navigation instead of typed into the search"
    );
    // The block must not swallow Enter/Esc, which are handled further down.
    assert!(
        !block.contains("KeyCode::Enter =>") && !block.contains("KeyCode::Esc =>"),
        "the nav block swallows Enter or Esc"
    );

    // The search line is what makes the query visible.
    assert!(
        lib.contains("let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());"),
        "render_libraries does not read the picker's query, so a search is invisible"
    );
    // The cursor needs its own marker: the old highlight keyed off
    // `library_category`, so it never moved.
    assert!(
        lib.contains("scroll_start + row == sel"),
        "the highlight is not keyed off the cursor"
    );
}

/// The Spotify charts provider is gone, and nothing may quietly reintroduce it.
///
/// It read editorial playlist tracklists through `/playlists/{id}` and
/// `/playlists/{id}/tracks`. Spotify removed both for developer-mode
/// integrations in 2026, so the source registered as `configured` and then
/// returned `no charts available` on every drill-down -- a permanently empty
/// entry in a menu that looked populated. Verified against the live daemon
/// before removal: `charts_list(apple)` returned 18 charts while
/// `charts_list(spotify)` returned "no charts available".
#[test]
fn no_spotify_charts_provider() {
    let registry = include_str!("../../gtmd/src/providers/charts/mod.rs");
    let daemon = include_str!("../../gtmd/src/daemon/mod.rs");
    let cmd = include_str!("../../gtmd/src/providers/spotify/cmd.rs");

    for (name, src) in [
        ("charts/mod.rs", registry),
        ("daemon/mod.rs", daemon),
        ("spotify/cmd.rs", cmd),
    ] {
        for gone in ["add_spotify", "ensure_spotify", "SpotifyCharts"] {
            assert!(
                !src.contains(gone),
                "{name} still references {gone}: the endpoint it needs was removed by Spotify"
            );
        }
    }

    // The provider file itself is deleted, and the registry registers only the
    // free feed -- so charts work with no account linked.
    assert!(
        registry.contains("pub fn add_free_defaults"),
        "the free chart provider must still be registered"
    );
    assert!(
        !registry.contains("mod spotify;"),
        "the spotify chart module is still declared"
    );
}

/// The up-next card must resolve its cover by path, not by track id.
///
/// A provider track (Spotify, YouTube) has no row in the local library, so
/// its `id` is not a library id and collides with whatever local track holds
/// that number. The cover lookup matched on `id` alone, so the up-next card
/// rendered the currently playing track's artwork. Both ends are asserted
/// here: the client sends the path, and the daemon prefers it.
#[test]
fn upnext_cover_is_resolved_by_path_not_id() {
    let cover = include_str!("../src/app/cover.rs");
    let daemon_cover = include_str!("../../gtmd/src/daemon/cover.rs");

    // The client must pass the path; `art().cover(id)` alone is the bug.
    let start = cover.find("pub fn start_upnext").expect("no start_upnext");
    let block = &cover[start..start + 2000];
    assert!(
        block.contains("cover_for(tid, cover_path)"),
        "start_upnext does not send the cover path"
    );
    assert!(
        !block.contains("art().cover(tid)"),
        "start_upnext still looks the cover up by id alone"
    );

    // The daemon must prefer a path match over an id match.
    assert!(
        daemon_cover.contains("let by_path = track_path.and_then(|p| {"),
        "the known-cover lookup does not try the path first"
    );
    assert!(
        daemon_cover.contains("None if track_id == 0 => None,"),
        "a non-zero id is still consulted ahead of the path"
    );
}

/// List rows must not print a bracketed `[stream]` where a duration goes.
///
/// The link glyph already marks a remote row, and `[stream]` sat in the
/// duration column looking like a length. Search rows lost the same brackets
/// around their kind tag, which put two sets of square brackets on one row.
#[test]
fn list_rows_have_no_bracketed_stream_suffix() {
    let queue = include_str!("../src/ui/pickers/queue.rs");
    let spotify = include_str!("../src/ui/pickers/mod.rs");

    assert!(
        !queue.contains("\"stream\".to_string()"),
        "the queue still builds a `[stream]` suffix"
    );
    // A remote row prints no tag at all; only `live` and durations survive.
    assert!(
        queue.contains("} else if remote {\n                None"),
        "the remote branch no longer suppresses the tag"
    );

    for gone in [
        "Some(\"[Album]\")",
        "Some(\"[Artist]\")",
        "Some(\"[Playlist]\")",
    ] {
        assert!(
            !spotify.contains(gone),
            "search rows still wrap the kind tag in brackets: {gone}"
        );
    }
    assert!(
        spotify.contains("Some(SpotifySearchKind::Album) => Some(\"Album\"),"),
        "the kind tag lost its unbracketed form"
    );
}

/// The lyrics header needs its own cover protocol, not the now-playing one.
///
/// Both panes can be on screen in the same frame, and one `StatefulProtocol`
/// rendered twice writes into the same cell buffer twice, so each pane would
/// draw part of the other. The state must therefore be per-pane.
#[test]
fn lyrics_cover_has_its_own_protocol() {
    let app = include_str!("../src/app/mod.rs");
    let cover = include_str!("../src/app/cover.rs");
    let chrome = include_str!("../src/ui/chrome.rs");

    assert!(
        app.contains("pub lyrics_cover: NowPlayingCoverState,"),
        "App has no separate cover state for the lyrics pane"
    );
    // Built in the same place as the now-playing one, from the same bytes.
    assert!(
        cover.contains("self.lyrics_cover.stateful = Some(picker.new_resize_protocol(img2))"),
        "the lyrics protocol is not built alongside the now-playing one"
    );
    // The renderer must take the lyrics one, not the shared now-playing one.
    assert!(
        chrome.contains("app.lyrics_cover.stateful.as_mut()"),
        "the lyrics header does not use its own protocol"
    );
    assert!(
        !chrome.contains("app.np_cover.stateful.as_mut(),\n                        app.np_cover.image.as_deref(),\n                        app.theme.fg_dim,\n                        Some(\" \\u{266b} \"),\n                    );\n                }\n\n                let para"),
        "the lyrics header still borrows the now-playing protocol"
    );
}

/// The now-playing cover and the left pane's card must place their artwork the
/// same way, or the two images sit at different heights in the same view.
#[test]
fn covers_are_placed_identically_in_both_panes() {
    let chrome = include_str!("../src/ui/chrome.rs");

    // The left pane's card centres horizontally in its area and takes the
    // area's own top edge.
    assert!(
        chrome.contains("x: area.x + cover_hpad,"),
        "the left card no longer centres its cover"
    );
    // The now-playing pane now does the same, with no `y + 1` nudge.
    assert!(
        chrome.contains("x: col.x + col.width.saturating_sub(cw) / 2,"),
        "the now-playing cover is not centred in its column"
    );
    assert!(
        chrome.contains("y: col.y + col.height.saturating_sub(ch) / 2,"),
        "the now-playing cover is not centred vertically"
    );
    // A leftover top-anchored variant would be the old geometry.
    assert!(
        !chrome.contains("y: hchunks[0].y + 1,"),
        "the old top-anchored now-playing cover rect is back"
    );
}

/// Three crossfade defects, all playback-visible.
///
/// PulseAudio muted the incoming stream, the `Finished` path advanced twice,
/// and two racing tasks could each advance a track. None of them is visible
/// without listening, so they are pinned in source.
#[test]
fn crossfade_advances_exactly_once_at_full_volume() {
    let pulse = include_str!("../src/audio/pulse.rs");
    let mixer = include_str!("../src/audio/mixer.rs");
    let daemon = include_str!("../../gtmd/src/daemon/mod.rs");

    // 1. The swap must not carry the ramped volume across. By the time a
    //    crossfade ends, `step_crossfade` has eased the outgoing stream to 0.
    assert!(
        !pulse.contains("let vol = self.get_mixer_volume();"),
        "swap_active_standby still reads the outgoing (ramped-to-zero) volume"
    );
    assert!(
        pulse.contains(
            "let vol = volume_from_ratio(volume_ratio(self.user_volume.load(Ordering::SeqCst)));"
        ),
        "the swap does not use the user's volume"
    );
    // The ALSA mixer was always right; PulseAudio now matches it.
    assert!(
        mixer.contains("volume_ratio(self.volume.load(Ordering::SeqCst))"),
        "the ALSA mixer lost its user-volume read"
    );

    // 2. `finish_crossfade` advances by itself, so the Finished path must not
    //    also call `Cmd::next`.
    let finished = daemon
        .find("let was_crossfading")
        .expect("no Finished branch");
    let block = &daemon[finished..finished + 1600];
    assert!(
        block.contains("Self::finish_crossfade(&inner).await;"),
        "the Finished path no longer finishes the crossfade"
    );
    assert!(
        !block.contains("Cmd::next(&inner).await"),
        "the Finished path still advances a second time after finish_crossfade"
    );

    // 3. The claim must be a `take`, so a second racing task sees it gone.
    assert!(
        daemon.contains("if inner.crossfade_loaded_for.lock().await.take().is_none() {"),
        "finish_crossfade does not claim the crossfade latch atomically"
    );
}

/// The up-next card's countdown must follow the real crossfade setting.
///
/// It was built from a field initialised to 6 and never written again, so the
/// card's length ignored the user's crossfade duration entirely.
#[test]
fn upnext_countdown_follows_the_crossfade_setting() {
    let cover = include_str!("../src/app/cover.rs");
    let app = include_str!("../src/app/mod.rs");

    assert!(
        !app.contains("pub crossfade_duration: u8,"),
        "the hardcoded crossfade duration field is back"
    );
    assert!(
        !cover.contains("self.crossfade_duration as f64"),
        "start_upnext reads the hardcoded duration again"
    );
    let start = cover.find("pub fn start_upnext").expect("no start_upnext");
    let block = &cover[start..start + 900];
    assert!(
        block.contains(".filter(|c| c.enabled)"),
        "the countdown ignores whether crossfade is enabled"
    );
    assert!(
        block.contains("map_or(0.0, |c| c.duration_secs as f64)"),
        "the countdown does not read the configured duration"
    );
    assert!(
        block.contains("let total_secs = cf_secs + 3.0;"),
        "the countdown window no longer matches the daemon's"
    );
}
