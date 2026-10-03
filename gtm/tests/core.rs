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
        image_url: Some("https://example.com/art.jpg".into()),
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
        track: Box::new(sample_track()),
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
        track: Box::new(sample_track()),
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

    // Charts resolve on how deep the drill-down is, so the arm is a match on
    // the level rather than a single kind. Every arm still has to be a chart
    // kind: a fall-through to `Track` here is the bug this test exists for.
    for kind in ["ChartSource", "Chart", "ChartTrack"] {
        assert!(
            body.contains(&format!("TrackInfoKind::{kind}")),
            "category 12 (Top Charts) must not fall through to TrackInfoKind::Track \
             at any level, and {kind} is missing:\n{body}"
        );
    }
    assert!(
        body.contains("12 => match (self.charts.selected_source, self.charts.selected_chart)"),
        "category 12 no longer resolves per level:\n{body}"
    );

    // Radio rows are virtual `radio://` stations, not library tracks, so it
    // needs an arm for the same reason.
    assert!(
        body.contains("6 => TrackInfoKind::RadioStation"),
        "category 6 (Radio) must not fall through to TrackInfoKind::Track:\n{body}"
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

    // Two free sources, both registered with no account linked.
    assert!(
        registry.contains("mod deezer;"),
        "the deezer chart provider is not declared"
    );
    assert!(
        registry.contains("Box::new(deezer::DeezerCharts::new())"),
        "the deezer chart provider is never registered, so its charts are unreachable"
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

/// The lyrics header shares the now-playing cover protocol, and does so on
/// purpose: one image, two draws.
///
/// The two panes used to keep separate `StatefulProtocol`s built from the same
/// bytes, on the theory that drawing one twice would garble it. They are the
/// same picture at the same size, so the second protocol cost a decode per pane
/// and could not drift from the first. Sharing is the fix, but only while both
/// panes really do ask for the same geometry — so that is what is pinned here.
#[test]
fn lyrics_cover_shares_the_now_playing_protocol() {
    let app = include_str!("../src/app/mod.rs");
    let cover = include_str!("../src/app/cover.rs");
    let chrome = include_str!("../src/ui/chrome.rs");

    // One state, not two. A second one would decode the same bytes twice.
    assert!(
        !app.contains("lyrics_cover"),
        "a second per-pane cover state is back"
    );
    // Built once, from the track's own art.
    assert!(
        cover
            .contains("Ok(img) => self.np_cover.stateful = Some(picker.new_resize_protocol(img)),"),
        "the now-playing protocol is not built from the fetched art"
    );
    // The lyrics header draws it, centred in the same column the left pane
    // uses, so the two images land at the same height.
    assert!(
        chrome.contains("app.np_cover.stateful.as_mut(),\n                        app.np_cover.image.as_deref(),\n                        app.theme.fg_dim,\n                        Some(\" \\u{266b} \"),"),
        "the lyrics header does not draw the shared protocol"
    );
    assert!(
        chrome.contains("x: col.x + col.width.saturating_sub(cw) / 2,"),
        "the lyrics cover is no longer centred in its column"
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

/// Every cover is centred on the size its protocol will actually draw.
///
/// `ratatui_image` fits the image and never upscales it past its own pixel size,
/// and every protocol draws from the top-left of the rect it is handed. So a box
/// sized for a full-bleed cover — the Zen screen, the info card — put the art in
/// its upper-left corner on any cover smaller than the box, and the two axes
/// were off by different amounts: high, and a little to the left. Handing the
/// protocol the rect it will fill is the only geometry that cannot drift.
#[test]
fn covers_centre_on_the_size_the_protocol_will_draw() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    assert!(
        chrome.contains(
            "f.render_stateful_widget(image, Render::cover_fit(area, protocol), protocol);"
        ),
        "the cover is handed the whole box instead of the rect it will fill"
    );

    let fit = chrome
        .split("pub(crate) fn cover_fit(")
        .nth(1)
        .expect("cover_fit is gone");
    let fit = &fit[..fit.find("pub(crate) fn cover(").expect("unterminated")];

    // The size the protocol will use, asked of the protocol: `StatefulImage` is
    // built with `Resize::Fit`, and a `Crop` or `Scale` here would describe a
    // rect the renderer never uses.
    assert!(
        squish(fit).contains(&squish(
            "let fit = protocol.size_for(Resize::Fit(None), Size::new(area.width, area.height));"
        )),
        "cover_fit does not ask the protocol for the size it will render"
    );
    // Both axes, and never outside the box it was given.
    assert!(
        squish(fit).contains(&squish(
            "x: area.x + area.width.saturating_sub(w) / 2, y: area.y + area.height.saturating_sub(h) / 2,"
        )),
        "the fitted cover is not centred on both axes"
    );
    assert!(
        squish(fit).contains(&squish("let w = fit.width.min(area.width).max(1);")),
        "the fitted cover can be wider than its box"
    );
    assert!(
        squish(fit).contains(&squish("let h = fit.height.min(area.height).max(1);")),
        "the fitted cover can be taller than its box"
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

/// Four scrobbling and state-persistence defects.
///
/// A scrobble that never lands and a state file that never loads both fail
/// silently, which is why they are pinned here rather than left to the log.
#[test]
fn scrobbling_and_state_persistence() {
    let lastfm = include_str!("../../gtmd/src/providers/lastfm/mod.rs");
    let daemon = include_str!("../../gtmd/src/daemon/mod.rs");
    let state = include_str!("../src/shared/state.rs");

    // 1. Last.fm's `timestamp` is when the track *started*. Stamping it with
    //    the submission time slid every play by the length of the track.
    assert!(
        lastfm.contains("chrono::Utc::now().timestamp() - played_secs.round() as i64"),
        "the scrobble timestamp is not backdated to the track's start"
    );
    assert!(
        !lastfm.contains("let timestamp = chrono::Utc::now().timestamp();"),
        "the scrobble timestamp is submission time again"
    );

    // 2. `Cmd::stop` cleared the track without scrobbling it, so stopping
    //    partway through lost the play.
    let stop = daemon
        .find("pub async fn stop(inner: &DaemonInner)")
        .expect("no stop");
    let block = &daemon[stop..stop + 1400];
    assert!(
        block.contains("Cmd::scrobble_track(inner, &track, played_secs).await;"),
        "stop no longer scrobbles the track it interrupts"
    );
    assert!(
        block.find("Cmd::scrobble_track").unwrap() < block.find("state.stop()?").unwrap(),
        "stop clears the track before scrobbling it"
    );

    // 3. A crossfaded track never reached Last.fm's "now playing", so the
    //    previous track sat there for the promoted track's whole length.
    let promoted = daemon
        .find("async fn report_promoted")
        .expect("no report_promoted");
    let block = &daemon[promoted..promoted + 1400];
    assert!(
        block.contains("Cmd::announce_now_playing(inner).await;"),
        "a crossfaded track is still never announced to Last.fm"
    );

    // 4. `SavedState::load` swallowed a parse failure, so one unrecognised key
    //    silently discarded the queue, the volume and the resume position.
    assert!(
        !state.contains("serde_json::from_str(&data).ok()"),
        "SavedState::load is discarding parse errors silently again"
    );
    assert!(
        state.contains("ignoring unreadable state file {}: {e}"),
        "a bad state file is not reported"
    );
    let saved = state.find("pub struct SavedState").expect("no SavedState");
    let block = &state[saved..saved + 1200];
    // `scrobble` was the field missing a default; every field now has one.
    for field in ["pub queue:", "pub volume:", "pub repeat:", "pub scrobble:"] {
        let at = block.find(field).expect(field);
        let before = &block[at.saturating_sub(40)..at];
        assert!(
            before.contains("#[serde(default)]"),
            "{field} can still fail the whole state file to deserialize"
        );
    }
}

/// The `--cover` flag is gone, and the half-block renderer with it.
///
/// It printed a fixed 16x8 grid that looked like a blurred thumbnail, and
/// making it render at full resolution was not a parameter change:
/// `ratatui-image`'s protocols write into a ratatui `Buffer`, and outside the
/// TUI there is no `Terminal` or `Buffer` to write into. Emitting real
/// artwork from the CLI meant hand-writing a Buffer-to-ANSI backend, which is
/// not something to ship unverified. The flag and the flag's only purpose
/// are removed rather than left rendering badly.
#[test]
fn no_cover_flag_or_halfblock_renderer() {
    let cli = include_str!("../src/cli.rs");
    let man = include_str!("../../docs/man/gtm.1.md");

    assert!(
        !cli.contains("fn cover_text("),
        "the half-block renderer is still there"
    );
    assert!(
        !cli.contains("async fn cover_str("),
        "the cover fetcher is still there"
    );
    assert!(!cli.contains("cover: bool,"), "the --cover flag is back");
    assert!(
        !cli.contains("last_frame_art") && !cli.contains("last_art"),
        "the per-tick art fetch in --stream mode is back"
    );
    // `status` still reports where the cover is; only the rendering is gone.
    assert!(
        cli.contains("\\x1b[1mCover:"),
        "status lost its Cover: line"
    );
    assert!(
        !man.contains("\\--cover"),
        "the man page still documents --cover"
    );
}

/// A picker's row count must match the rows its renderer draws.
///
/// The crossfade picker draws six rows (a "Duration" header plus the five
/// durations) but reported fourteen, so the cursor could be moved eight rows
/// past the end of the list. The renderer clamped it back for drawing while
/// the cursor kept counting, which reads as the list refusing to scroll --
/// navigation that does nothing rather than navigation that is wrong.
#[test]
fn picker_row_counts_match_their_renderers() {
    let app = include_str!("../src/app/mod.rs");
    let presets = include_str!("../src/ui/pickers/presets.rs");

    // 1 header + CROSSFADE_DURATIONS.len().
    assert!(
        presets.contains("rows.push(\" Duration \".to_string());"),
        "the crossfade picker lost its header row"
    );
    assert!(
        app.contains("PickerId::Crossfade => 6,"),
        "the crossfade picker's row count is wrong again"
    );
    assert!(
        app.contains("PickerId::Crossfade => 5,"),
        "the crossfade picker's max index is wrong again"
    );

    // The Enter arm addresses durations at rows 1..=5, which has to agree.
    let keys = include_str!("../src/app/keys.rs");
    let at = keys
        .find("PickerId::Crossfade =>")
        .expect("no crossfade arm");
    let block = &keys[at..at + 700];
    assert!(
        block.contains("(1..=5).contains(&sel)"),
        "the crossfade Enter arm's row range moved"
    );
}

/// Every built-in theme's list text must clear WCAG AA against its own pane
/// background.
///
/// `fg_dim` is the colour behind every label, hint, footer and secondary field,
/// and it was below 4.5:1 in eleven of the sixteen themes -- as low as 1.69:1
/// in Nord. The theme constructors are the right place to fix that: correcting
/// it at the call site would force `readable_fg` on every consumer and flatten
/// the dim/bright hierarchy the themes are built around.
#[test]
fn theme_dim_text_is_readable_in_every_builtin() {
    let theme = include_str!("../src/theme.rs");

    // The corrected values, so a theme edit that undoes one is caught here.
    for (name, field, hex) in [
        ("chadrula", "fg_dim", "0x8a91ae"),
        ("one_dark", "fg_dim", "0x90959e"),
        ("tokyonight", "fg_dim", "0x7d84a4"),
        ("catppuccin_mocha", "fg_dim", "0x848799"),
        ("gruvbox_dark", "fg_dim", "0x9c8e81"),
        ("nord", "fg_dim", "0x999faa"),
        ("rose_pine", "fg_dim", "0x848098"),
        ("everforest", "fg_dim", "0x99a097"),
        ("kanagawa", "fg_dim", "0x8a8982"),
        ("classic", "fg_dim", "0x858585"),
        ("monochrome", "fg_dim", "0x84858c"),
        ("solarized_dark", "fg_dim", "0x829298"),
        ("solarized_dark", "accent", "0x3a95d6"),
    ] {
        let start = theme
            .find(&format!("fn {name}() -> AppTheme"))
            .unwrap_or_else(|| panic!("no {name} theme constructor"));
        let body = &theme[start..start + 1400];
        assert!(
            body.contains(&format!("{field}: hex({hex})")),
            "{name}.{field} is no longer the value that clears 4.5:1"
        );
    }
}

/// `readable_fg` must return colours `contrast` can actually measure.
///
/// `contrast` computes relative luminance and only understands `Color::Rgb`;
/// every other variant collapses to 0.5. Returning `Color::Black` from
/// `readable_fg` therefore meant the function chose an endpoint by comparing
/// two values it could not measure, and returned a colour whose readability it
/// could not verify -- a terminal-dependent named colour at that.
#[test]
fn readable_fg_returns_measurable_colours() {
    let theme = include_str!("../src/theme.rs");
    let start = theme.find("pub fn readable_fg").expect("no readable_fg");
    let body = &theme[start..start + 2000];
    assert!(
        body.contains("const BLACK: Color = Color::Rgb(0, 0, 0);"),
        "readable_fg no longer pins its endpoints to explicit RGB"
    );
    assert!(
        body.contains("const WHITE: Color = Color::Rgb(255, 255, 255);"),
        "readable_fg no longer pins its endpoints to explicit RGB"
    );
    // The named variants are what could not be measured.
    assert!(
        !body.contains("Color::Black\n") && !body.contains("Color::White\n"),
        "readable_fg returns a named colour again"
    );
}

/// Completions must come from the real parsers, not a copy of them.
///
/// `gtm/build/completions.rs` used to be a hand-maintained duplicate of the
/// CLI, and the shipped scripts had drifted: `api`, `cli`, `lyrics` and
/// `stream` were missing from it. The duplicate is gone, so the guard is that
/// both binaries expose the generator and that no copy of the arg structs is
/// hiding in a build script.
#[test]
fn completions_are_generated_from_the_real_parsers() {
    let build = include_str!("../build.rs");
    let cli = include_str!("../src/cli.rs");
    let main = include_str!("../src/main.rs");
    let daemon_config = include_str!("../../gtmd/src/config.rs");
    let daemon_lib = include_str!("../../gtmd/src/lib.rs");

    // No duplicate CLI definition, and no include! of one.
    for gone in ["include!(", "GTM_GEN_COMPLETIONS", "mod completions"] {
        assert!(
            !build.contains(gone),
            "build.rs is back to carrying completion logic: {gone}"
        );
    }
    for (name, src) in [
        ("cli.rs", cli),
        ("main.rs", main),
        ("gtmd config.rs", daemon_config),
        ("gtmd lib.rs", daemon_lib),
    ] {
        assert!(
            !src.contains("#[derive(Parser)]") || name == "cli.rs",
            "{name} reintroduced a parallel arg struct"
        );
    }

    // Both binaries can emit a script, from their own command tree.
    assert!(cli.contains("pub completions: Option<clap_complete::Shell>"));
    assert!(daemon_config.contains("pub completions: Option<clap_complete::Shell>"));
    assert!(main.contains("clap_complete::generate(shell, &mut cmd, \"gtm\""));
    assert!(daemon_lib.contains("clap_complete::generate(shell, &mut cmd, \"gtmd\""));

    // And the generation happens before the daemon starts or the TUI launches,
    // so packaging needs neither.
    let at = daemon_lib.find("clap_complete::generate").unwrap();
    let cfg = daemon_lib.find("DaemonConfig::load").unwrap();
    assert!(at < cfg, "gtmd generates completions after loading config");
}

/// The Setup chooser and its services must agree on how many there are.
///
/// `Alt+X` already opened the Setup chooser, so Discord was added as a fourth
/// service rather than by taking the key: a second `Alt+X` binding would have
/// shadowed the first, and a service row the chooser cannot count to would be
/// unreachable.
#[test]
fn discord_is_a_setup_service_and_alt_x_is_not_duplicated() {
    let keymap = include_str!("../src/keymap.rs");
    let state = include_str!("../src/app/state.rs");
    let keys = include_str!("../src/app/keys.rs");
    let forms = include_str!("../src/ui/pickers/forms.rs");
    let run = include_str!("../src/app/run.rs");
    let icons = include_str!("../src/ui/icons.rs");

    // Exactly one Alt+X binding, and it is the Setup chooser. Counted by the
    // full key expression: a line-window match also counts the lines of every
    // neighbouring binding.
    let alt_x = "KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)";
    assert_eq!(
        keymap.matches(alt_x).count(),
        1,
        "Alt+X is bound more than once"
    );
    assert!(keymap.contains("OpenOverlay(PickerId::Setup)"));

    // Four services, counted consistently in the three places that care.
    assert!(state.contains(r#"let names = ["spotify", "lastfm", "youtube", "discord"];"#));
    assert!(state.contains("app.setup.selection.min(3)"));
    assert!(keys.contains("let n = 4;"));
    assert!(forms.contains("let services: [(&str, &str, String); 4] = ["));
    assert!(forms.contains(r#"("Discord", "rich presence""#));

    // And the chooser can open it.
    assert!(run.contains("Some(\"discord\") => {"));
    assert!(run.contains("self.pickers.open(PickerId::DiscordSetup);"));

    // A real brand glyph, not a stand-in, and the right one: U+F075E was
    // labelled `nf-md-discord` while being MDI's volume-minus, so the row drew
    // a speaker with a dash through it. The Discord glyph is U+F066F.
    assert!(
        icons.contains(r#""Discord" => Some("\u{f066f}")"#),
        "the Discord brand glyph is missing or is the wrong codepoint"
    );
    assert!(
        !icons.contains(r#""Discord" => Some("\u{f075e}")"#),
        "the Discord glyph is still volume-minus"
    );
    // And the emoji style has an arm for it, so the row is not a blank cell
    // before its label.
    assert!(
        icons.contains("\"Discord\" => \"\\u{1f4ac}\""),
        "the emoji icon style leaves the Discord row without a glyph"
    );
}

/// Zen's four bands, in order, and the progress indicator actually landing in
/// the last one.
///
/// `zen_now_playing` read the progress bar and the elapsed time out of
/// `vchunks[2]` — the artwork's own band. The bar drew over the top of the
/// cover, the time drew two rows into it, and the 3 rows reserved below were
/// never rendered to at all, which read as the cover floating too high above a
/// band of dead space. Whitespace is stripped so reformatting cannot hide a
/// reordering.
#[test]
fn zen_bands_are_header_cover_lyric_progress() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    let zen = chrome
        .split("fn zen_now_playing")
        .nth(1)
        .expect("zen_now_playing is gone");
    let zen = &zen[..zen
        .find("fn zen_lyric_line")
        .expect("zen_lyric_line is gone")];

    assert!(
        squish(zen).contains(&squish(
            ".constraints([Constraint::Length(2),Constraint::Min(0),Constraint::Length(1),Constraint::Length(3),Constraint::Length(3),])"
        )),
        "zen's bands are not header / cover / lyric / progress over a surface margin"
    );

    // Five bands, and the fifth is surface: without it the lyric line and the
    // progress indicator sit on the bottom edge of the terminal. Asserted
    // squished so the explanatory comment above the last constraint does not
    // have to be kept in step with this string.
    assert_eq!(
        squish(zen).matches("Constraint::Length(").count()
            + squish(zen).matches("Constraint::Min(").count(),
        5,
        "zen must reserve a band under the progress indicator"
    );

    // Each band is claimed by exactly one thing, and the cover's is not the
    // progress bar's.
    for (band, owner) in [
        ("0", "Render::zen_track_header(f, app, t, vchunks[0]);"),
        ("1", "let art_band = vchunks[1];"),
        ("2", "let lyric_rect = vchunks[2];"),
        ("3", "let prog = vchunks[3];"),
    ] {
        assert!(zen.contains(owner), "zen band {band} is no longer {owner}");
    }

    // The regression itself, in the shape it had.
    assert!(
        !squish(zen).contains(&squish("let prog = vchunks[2];")),
        "the progress bar is reading the artwork's band again"
    );
}

/// Tab has to swap the lyrics out of the results pane, not just move a focus
/// bar.
///
/// `lyrics_results_pane` was `lyrics.show && lyrics_area.is_none()` and never
/// consulted focus, so under 100 columns the moment `l` was on the library
/// results list was skipped for good and the lyrics sat over the pane
/// permanently. On narrow the list was additionally being drawn into
/// `panes[1]`, which is `Length(0)` while the left pane has focus, so it was
/// not on screen at all.
#[test]
fn lyrics_own_the_results_pane_only_while_focused() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let keys = include_str!("../src/app/keys.rs");

    assert!(
        chrome.contains(
            "let lyrics_results_pane = app.lyrics.show && lyrics_area.is_none() && app.lyrics.pane_focus;"
        ),
        "the results pane no longer yields to the lyrics when focus moves away"
    );
    // And the list is skipped, and the lyrics drawn, off that one flag.
    assert!(chrome.contains("if !lyrics_results_pane {"));
    assert!(chrome.contains("} else if lyrics_results_pane {"));

    // Every way of turning the lyrics on has to move focus into them, or `l`
    // puts the pane in the unfocused state and nothing appears. Checked per
    // occurrence: the transcript key and the palette action are the same
    // statement, so searching for the first would pass on the first alone.
    let mut sites = 0;
    let mut at = 0;
    while let Some(found) = keys[at..].find("self.lyrics.show = true;") {
        let start = at + found;
        let tail = &keys[start..start + 240];
        assert!(
            tail.contains("self.lyrics.pane_focus = true;"),
            "a site that turns the lyrics on does not focus them, so they do not appear"
        );
        sites += 1;
        at = start + 1;
    }
    assert_eq!(
        sites, 2,
        "expected the transcript key and the palette action"
    );

    // The `l` toggle, as a slice: a fixed character window measured how far the
    // explanatory comment happened to run.
    let l = keys
        .split("Some(KeyboardAction::FetchLyrics) => {")
        .nth(1)
        .expect("`l` is gone");
    let l = &l[..l
        .find("Some(KeyboardAction::")
        .expect("FetchLyrics arm is unterminated")];
    assert!(l.contains("self.lyrics.show = !self.lyrics.show;"));
    assert!(
        l.contains("self.lyrics.pane_focus = true;"),
        "`l` turns the lyrics on without focusing them, so they do not appear"
    );
    assert!(
        l.contains("self.lyrics.pane_focus = false;"),
        "`l` leaves the lyrics pane focused after hiding them"
    );
}

/// The lyrics header is a cover and nothing else, and only on one column.
///
/// The pane carried a 3-row cover *and* the track's title and artist in every
/// layout. Beside the now-playing band that repeated the pane next to it, and on
/// one column it took five rows off a band that is five rows tall — while the
/// "LYRICS" label above it named a pane the band had already titled. The
/// artwork is the one thing the band cannot spare, so it comes here instead,
/// The lyrics pane is lyrics and nothing else, in every layout.
///
/// It carried a cover image in one layout and a "LYRICS" label plus a left rule
/// in another, neither of which told the listener anything: the track is named
/// by the now-playing band in one column and by the neighbouring pane in three,
/// and the artwork was the same bytes decoded and uploaded a second time for a
/// surface whose whole job is to be read.
#[test]
fn the_lyrics_pane_carries_no_chrome() {
    let chrome = include_str!("../src/ui/chrome.rs");

    let pane = chrome
        .split("pub(crate) fn lyrics_pane(")
        .nth(1)
        .expect("lyrics_pane is gone");
    let pane = &pane[..pane
        .find("pub(crate) fn lyrics_body(")
        .expect("lyrics_pane is unterminated")];

    assert!(
        !pane.contains("Render::cover"),
        "the lyrics pane still draws a cover image"
    );
    assert!(
        !pane.contains("pane_header"),
        "the lyrics pane still draws a header or a border"
    );
    for gone in ["lyrics.title", "lyrics.artist", "app.terminal_cols"] {
        assert!(!pane.contains(gone), "the lyrics header still reads {gone}");
    }

    // One call site shape, so neither layout can reintroduce a fit-dependent
    // header by way of the argument that used to select it.
    assert_eq!(
        chrome
            .matches("Render::lyrics_pane(f, lyrics_area, app)")
            .count()
            + chrome
                .matches("Render::lyrics_pane(f, lyrics, app)")
                .count(),
        2,
        "the lyrics pane is no longer called without a layout argument"
    );
    assert!(
        !chrome.contains("LyricsFit"),
        "the per-layout fit argument is back"
    );
}

/// The now-playing band starts at the results column, and the library list runs
/// the full height beside it.
///
/// The band used to sit above the library column, so the cover, the title and
/// the progress were a screen away from the list they belonged to, and the
/// category list started a third of the way down the screen for no reason.
#[test]
fn now_playing_starts_at_the_results_column() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    assert!(
        chrome.contains("let (np_area, lib_area, results_area) = if is_narrow {"),
        "the three panes are not split in one place any more"
    );
    // Wide: the library column off the left, then the band stacked over the
    // results in what is left. Narrow keeps the band across the full width
    // because the column below it collapses to nothing when it holds focus.
    assert!(
        squish(chrome).contains(&squish(
            "let h = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(lib_width), Constraint::Min(0)]).split(left_area);"
        )),
        "the library column is not split off the left first"
    );
    assert!(
        squish(chrome).contains(&squish(
            "let v = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(np_height), Constraint::Min(1)]).split(h[1]);"
        )),
        "the band is not stacked over the results"
    );

    // Each pane is drawn from its own rect, and the band from the one that
    // starts where the results start.
    for pane in ["np_area", "lib_area", "results_area"] {
        assert!(
            squish(chrome).contains(&squish(&format!("Render::pane_header(f, {pane}, app,"))),
            "{pane} is not drawn from its own rect"
        );
    }

    // The band is narrower than it was, so the cover has to give up the columns
    // the title, artist, album and progress need.
    assert!(
        chrome.contains("let cover_h = cover_h.min(inner.width.saturating_sub(20) / 2).max(3);"),
        "the now-playing cover is not capped by the width left for the details"
    );
}

/// The visualizer is a Zen surface, not a third of the now-playing band.
///
/// It drew into a third of the band, which is the same rows the cover and the
/// progress were on, and the frame loop spun at 60fps for it whether or not it
/// The visualizer has no on/off switch: Zen or daydreaming decides whether it
/// is on screen, and the extension config is the only thing that turns it off.
///
/// A toggle was wrong on its own terms. The visualizer is a fullscreen surface,
/// so "off" was never the interesting state — it was a way to reach a mode the
/// key that enters Zen already reaches. Worse, off was the default, so a fresh
/// install showed the feature the configuration file documents as enabled and
/// the key that is not in the man page turned it on.
#[test]
fn the_visualizer_has_no_toggle() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let run = include_str!("../src/app/run.rs");
    let keys = include_str!("../src/app/keys.rs");
    let map = include_str!("../src/keymap.rs");
    let viz = include_str!("../src/visualizer.rs");

    let lib = chrome
        .split("pub(crate) fn library(")
        .nth(1)
        .expect("library() is gone");
    let lib = &lib[..lib
        .find("pub(crate) fn footer(")
        .expect("library() is unterminated")];

    assert!(
        !lib.contains("visualizer"),
        "the library view still draws the visualizer"
    );
    assert!(
        !chrome.contains("show_vis"),
        "the band still reserves rows for it"
    );
    // No action, no binding, no palette entry, no enabled flag to consult.
    assert!(!map.contains("ToggleVisualizer"), "the action is back");
    assert!(!keys.contains("ToggleVisualizer"), "a dispatch arm is back");
    assert!(!viz.contains("pub enabled"), "the enabled flag is back");
    assert!(!viz.contains("pub fn toggle("), "toggle() is back");
    assert!(
        !chrome.contains("press Ctrl+V to enable"),
        "the Zen surface still tells the user to press the removed key"
    );

    // What replaced it: the frame loop pays for the frames only when a surface
    // that draws it is up, and it consults the extension switch rather than a
    // second one.
    assert!(chrome.contains("pub(crate) fn zen_visualizer("));
    assert!(
        run.contains("&& (self.daydreaming"),
        "daydreaming does not keep the frame loop awake"
    );
    assert!(
        run.contains("self.zen_surface == ZenSurface::Visualizer"),
        "Zen no longer decides to animate for the visualizer"
    );
    assert!(
        run.contains("!self.extensions.is_disabled(ExtensionId::Visualizer)"),
        "the animation no longer respects the extension switch"
    );
}

/// Zen has three surfaces again, and the lyrics are one of them.
#[test]
fn zen_cycles_now_playing_lyrics_and_the_visualizer() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let state = include_str!("../src/app/state.rs");
    let app = include_str!("../src/app/mod.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    for surface in ["NowPlaying", "Lyrics", "Visualizer"] {
        assert!(
            state.contains(&format!("    {surface},")),
            "ZenSurface::{surface} is gone"
        );
    }
    // A three-arm cycle in both directions, or Tab lands on nothing.
    for f in ["next", "prev"] {
        let body = state
            .split(&format!("pub(crate) fn {f}(self) -> ZenSurface {{"))
            .nth(1)
            .unwrap_or_else(|| panic!("ZenSurface::{f} is gone"));
        let body = &body[..body.find("\n    }").expect("unterminated")];
        for surface in ["NowPlaying", "Lyrics", "Visualizer"] {
            assert!(
                body.contains(&format!("ZenSurface::{surface} =>")),
                "{f} does not reach {surface}"
            );
        }
    }

    // Every surface is dispatched, and the lyrics render through the same body
    // as the docked pane rather than a copy of it.
    for arm in [
        "ZenSurface::NowPlaying => Render::zen_now_playing(f, area, app),",
        "ZenSurface::Lyrics => Render::zen_lyrics(f, area, app),",
        "ZenSurface::Visualizer => Render::zen_visualizer(f, area, app),",
    ] {
        assert!(chrome.contains(arm), "zen does not dispatch {arm}");
    }
    assert!(
        chrome.contains("Render::lyrics_body(f, body, app, lyrics);"),
        "the zen lyrics do not share the pane's body"
    );
    // And Zen is painted on its own background, lifted off the app surface out
    // of the reactive palette — the app's own surface is already washed with
    // it, so on a fullscreen surface the two were indistinguishable.
    assert!(
        squish(chrome).contains(&squish(
            "let bg = if zen { app.zen_bg() } else { app.surface_bg() };"
        )),
        "the zen surfaces are still painted on the app background"
    );
    assert!(app.contains("pub fn zen_bg(&self)"), "App::zen_bg is gone");
    assert!(
        app.contains("self.reactive_palette.filter(|_| self.reactive_theme)"),
        "the zen background is no longer taken from the reactive palette"
    );
}

/// The now-playing cover uses the whole pane on narrow screens.
///
/// The 2-row padding cost half the art there: on the 5-row pane a narrow
/// terminal gets, `avail_h` was 2 and the cover 4x2 cells.
#[test]
fn now_playing_cover_fills_a_narrow_pane() {
    let chrome = include_str!("../src/ui/chrome.rs");

    assert!(
        chrome.contains(
            "let avail_h = if is_narrow {\n                    inner.height\n                } else {\n                    inner.height.saturating_sub(2)\n                };"
        ),
        "the narrow now-playing cover is padded again"
    );
}

/// Narrow screens float the track-info card over the list instead of docking
/// it, and it has to be painted after the rows.
///
/// The card was a left-pane info block, which on a one-pane layout cost a
/// sixth of the rows it was describing. Floating it is what it used to do
/// before it was folded into the pane, and the point of the assertion on
/// ordering is that a float drawn before the list is a float under the list.
///
/// The two halves of the size are the defect worth pinning. The box was the
/// docked card's geometry — 24 columns and `info_block_h()` rows — while the
/// artwork inside was sized off the height the float was left with, so the box
/// came out eight columns wider than the art it held and taller than its own
/// contents. And the list was sized to the whole pane, so it scrolled rows
/// Narrow screens dock the track-info card under the list, and the list gives up
/// the rows it takes.
///
/// It used to float over them, anchored to the bottom-right corner: a card
/// narrower than the pane but taller than the space it left, so it covered rows
/// that could be neither read nor clicked, while its height was a second
/// independent estimate of a size its own contents already determined.
#[test]
fn narrow_docks_the_card_and_the_list_yields_its_rows() {
    let chrome = include_str!("../src/ui/chrome.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    assert!(
        squish(chrome).contains(&squish(
            "let dock_card = is_narrow && !lyrics_results_pane && app.show_preview && app.track_popup_visible;"
        )),
        "the docked card is not gated to narrow screens with the list on screen"
    );
    // Docked in the library column's own info block on wide screens still.
    assert!(
        squish(chrome).contains(&squish(
            "} else if !is_narrow { Render::info_in_pane(f, info_sep_area, left_info_area, app);"
        )),
        "the card is no longer docked on wide screens"
    );

    let lib = chrome
        .split("pub(crate) fn library(")
        .nth(1)
        .expect("library() is gone");
    let lib = &lib[..lib
        .find("pub(crate) fn footer(")
        .expect("library() is unterminated")];

    // Full width, at the bottom of the pane it is drawn into.
    assert!(
        squish(chrome).contains(&squish(
            "let rect = Rect { x: area.x, y: area.y + area.height.saturating_sub(h), width: area.width, height: h, };"
        )),
        "the card is not a full-width strip at the bottom of the pane"
    );
    assert!(
        !chrome.contains("pub(crate) fn floating_card("),
        "the floating card is back"
    );
    assert!(
        !lib.contains("f.render_widget(Clear,"),
        "the card is still cleared as a float rather than filled as a pane"
    );

    // Height derived from the artwork, so the box and its contents cannot
    // disagree the way the float's two estimates did.
    assert!(
        squish(chrome).contains(&squish(
            "let art = area.height.saturating_sub(INFO_FIELDS_H + 4).min(DOCK_ART_H).min(area.height / 4).max(2);"
        )),
        "the docked card is not sized from the artwork it holds"
    );

    // The list gives up the card's rows, on every category branch: one shared
    // budget rather than fifteen copies of `height - 3`.
    assert!(
        squish(chrome).contains(&squish(
            "let window_rows = || results_area.height.saturating_sub(3 + dock_h) as usize;"
        )),
        "the list is not sized around the docked card"
    );
    assert!(
        !lib.contains("let reserve = 3usize;"),
        "a category branch is still sizing its own window, without the card's rows"
    );
    assert_eq!(
        squish(lib)
            .matches(&squish("let available = window_rows();"))
            .count(),
        squish(lib)
            .matches(&squish("app.viewport_items = available;"))
            .count(),
        "a category branch sets a viewport without the shared budget"
    );

    // And the mouse zones subtract it too. They used to stop at the pane's own
    // rows, so every hit zone under the card's top edge belonged to a row the
    // card was covering: scrollable to, unclickable.
    assert!(
        squish(lib).contains(&squish(
            "let avail = right_inner.height.saturating_sub(2).saturating_sub(dock_h) as usize;"
        )),
        "the mouse hit zones still cover rows under the docked card"
    );

    // The card's own cover gate has to be the box it was handed.
    assert!(
        chrome.contains(
            "let can_cover = !no_image_protocol() && area.width >= 6 && area.height >= 8;"
        ),
        "the info card still demands the docked card's width before drawing art"
    );
}

/// Every chart level and the Radio list must produce an info card, and a chart's
/// rows must have their artwork fetched and warmed rather than only the one the
/// cursor is on.
///
/// Charts returned `ChartTrack` at all three levels while the fields for that
/// kind read `chart_tracks`, which is empty until a chart is opened: the source
/// list and the chart list had no card. Radio fell through to `Track`, whose
/// rows are virtual `radio://` stations that `filtered_tracks` clears, so it had
/// no card either.
#[test]
fn charts_and_radio_rows_all_describe_themselves() {
    let cover = include_str!("../src/app/cover.rs");
    let text = include_str!("../src/ui/text.rs");
    let state = include_str!("../src/app/state.rs");
    let run = include_str!("../src/app/run.rs");
    let search = include_str!("../src/app/search.rs");
    let squish = |s: &str| s.split_whitespace().collect::<String>();

    for kind in ["ChartSource", "Chart", "RadioStation"] {
        assert!(
            state.contains(&format!("    {kind},")),
            "TrackInfoKind::{kind} is gone"
        );
    }

    // The level decides the kind, and every level is covered.
    assert!(squish(cover).contains(&squish(
        "12 => match (self.charts.selected_source, self.charts.selected_chart) { (None, _) => TrackInfoKind::ChartSource, (Some(_), None) => TrackInfoKind::Chart, (Some(_), Some(_)) => TrackInfoKind::ChartTrack, },"
    )));
    assert!(squish(cover).contains(&squish("6 => TrackInfoKind::RadioStation,")));

    // Each kind has fields, or the card is `None` and nothing renders.
    for kind in ["ChartSource", "Chart", "RadioStation"] {
        assert!(
            text.contains(&format!("TrackInfoKind::{kind} => {{")),
            "no fields for {kind}: the card would be None and render nothing"
        );
    }

    // Validity is per kind, so an empty list at any level hides the card
    // instead of indexing a list that is not there.
    for probe in [
        "TrackInfoKind::ChartSource => self.list_pos() < self.charts.sources.len(),",
        "TrackInfoKind::Chart => self.list_pos() < self.charts.charts.len(),",
        "TrackInfoKind::ChartTrack => self.list_pos() < self.charts.chart_tracks.len(),",
        "TrackInfoKind::RadioStation => self.list_pos() < self.radio.custom.len(),",
    ] {
        assert!(squish(cover).contains(&squish(probe)), "missing: {probe}");
    }

    // A chart's own artwork, and a chart row's, go through the URL fetch; a
    // station resolves to none and its card is text.
    assert!(
        cover.contains("TrackInfoKind::Chart => self.charts.charts.get(pos)?.cover_url.clone(),")
    );
    assert!(cover.contains(
        "TrackInfoKind::ChartTrack => self.charts.chart_tracks.get(pos)?.cover_url.clone(),"
    ));
    assert!(cover.contains("pub(crate) fn fetch_url_cover(&mut self, url: Option<String>) {"));

    // The rows of a loaded chart are warmed around the cursor, not just the one
    // under it: the card shows a single row's art, so without this every step
    // of a scroll is a blank card.
    assert!(cover.contains("pub fn preload_chart_covers(&self) {"));
    // Warmed from every cursor move, keyed on the loaded rows rather than on a
    // category index: the chart list is three levels deep and only the last has
    // rows, so a category test either fired on the wrong level or not at all.
    assert!(
        cover.contains("self.preload_chart_covers();\n        let pos = self.list_pos();"),
        "the chart warm is no longer part of every cursor move's preload"
    );
    assert!(
        !cover.contains("if self.library_category == 12 {"),
        "the chart warm is gated on a category index again"
    );

    // Arriving at a list builds the card. Charts load asynchronously, so the
    // two list replies have to ask for it too.
    assert!(search.contains("self.update_track_popup();"));
    let lists = run.matches("self.update_track_popup();").count();
    assert!(
        lists >= 3,
        "only {lists} chart replies rebuild the card; the source and chart lists load async"
    );
}

/// One script generates completions, and every packaging caller uses it.
///
/// Completions used to be a side effect of `cargo build` keyed on
/// `GTM_GEN_COMPLETIONS`, set in six places. Replacing that with binaries that
/// emit their own scripts meant editing all six, and `release.yml` was missed:
/// the nightly build then died at `cp: cannot stat 'artifacts/completions/*'`
/// because nothing populated the directory any more. The invariant is the point
/// — a caller that installs from `artifacts/completions` without invoking the
/// generator is the whole bug.
#[test]
fn completion_consumers_all_invoke_the_one_generator() {
    let script = include_str!("../../scripts/build/completions.sh");

    // The generator asks the binaries, so it cannot drift from the parsers.
    assert!(script.contains("--completions"));
    assert!(
        script.contains("for bin in gtm gtmd; do"),
        "the generator no longer covers both binaries"
    );
    // clap's zsh script is a completion *function*, so it is installed as
    // `_gtm`. Emitting `gtm.zsh` or `gtm._` matches nothing that consumes it,
    // and the .deb staging resolves the asset by name and fails the build.
    assert!(
        script.contains(">\"$dest/_$bin\""),
        "the zsh output is no longer written as _$bin"
    );
    for name in ["$bin.bash", "$bin.fish", "$bin.elv", "$bin.ps1"] {
        assert!(
            script.contains(&format!(">\"$dest/{name}\"")),
            "no output for {name}"
        );
    }

    // Every consumer that reads the directory must also produce it.
    for (what, path) in [
        ("Makefile", include_str!("../../Makefile")),
        (
            "release.yml",
            include_str!("../../.github/workflows/release.yml"),
        ),
        ("PKGBUILD", include_str!("../../dist/arch/PKGBUILD")),
        ("gtmd.spec", include_str!("../../dist/rpm/gtmd.spec")),
        (
            "musl-in-container.sh",
            include_str!("../../scripts/build/musl-in-container.sh"),
        ),
        (
            "arch-in-container.sh",
            include_str!("../../scripts/build/arch-in-container.sh"),
        ),
        ("flake.nix", include_str!("../../flake.nix")),
    ] {
        if path.contains("artifacts/completions") {
            assert!(
                path.contains("completions.sh"),
                "{what} installs from artifacts/completions but never generates it"
            );
        }
    }

    // Every family whose packaging copies from the directory must be one the
    // generate step actually runs for. Android was excluded from it while its
    // archive still copied the directory, so the copy had nothing to copy.
    let rel = include_str!("../../.github/workflows/release.yml");
    let step = |name: &str| {
        rel.split(&format!("- name: {name}"))
            .nth(1)
            .unwrap_or_else(|| panic!("{name} is gone"))
            .split("run:")
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let consumes: Vec<&str> = ["debian", "macos", "arch", "android"].into();
    let gated = step("Generate shell completions");
    for family in consumes {
        assert!(
            !gated.contains(&format!("matrix.family == '{family}'")),
            "the generate step excludes {family}, whose packaging copies artifacts/completions"
        );
    }
    // And the two it does exclude generate inside their build containers.
    for family in ["musl", "arch-arm"] {
        assert!(
            gated.contains(&format!("matrix.family != '{family}'")),
            "the generate step no longer excludes {family}"
        );
    }
    assert!(
        step("Build musl binaries + packages (Alpine container)").is_empty()
            || include_str!("../../scripts/build/musl-in-container.sh").contains("completions.sh"),
        "musl generates neither in the workflow nor in its container"
    );

    // And the retired mechanism is gone everywhere, not just from build.rs.
    for (what, path) in [
        ("build.rs", include_str!("../build.rs")),
        ("Makefile", include_str!("../../Makefile")),
        (
            "release.yml",
            include_str!("../../.github/workflows/release.yml"),
        ),
        ("PKGBUILD", include_str!("../../dist/arch/PKGBUILD")),
        ("gtmd.spec", include_str!("../../dist/rpm/gtmd.spec")),
        (
            "musl-in-container.sh",
            include_str!("../../scripts/build/musl-in-container.sh"),
        ),
        (
            "arch-in-container.sh",
            include_str!("../../scripts/build/arch-in-container.sh"),
        ),
        ("flake.nix", include_str!("../../flake.nix")),
    ] {
        assert!(
            !path.contains("GTM_GEN_COMPLETIONS"),
            "{what} still sets GTM_GEN_COMPLETIONS, which no longer does anything"
        );
    }
}

/// A persisted audio setting that nothing replays into the mixer is not a
/// setting, it is a line in a JSON file.
///
/// Pre-gain was the third one. `pre_gain_db` lived in `AudioSettings`, had a
/// setter, an IPC request and a `pre_gain_changed` event — and the only thing
/// the setter did was write state and announce the change. Nothing multiplied
/// any samples by it, on any backend, at any point. EQ, reverb and the audio
/// device were all in the same position, and the device was the only one that
/// happened to work, because the mixer factory happened to replay it.
///
/// So this pins the two halves that have to stay together: the value reaches
/// the sample path, and it is replayed at startup rather than waiting for the
/// user to touch the control.
#[test]
fn pre_gain_reaches_the_samples_and_survives_a_restart() {
    let decoder = include_str!("../src/audio/decoder.rs");
    let mixer = include_str!("../src/audio/mixer.rs");
    let daemon = include_str!("../../gtmd/src/daemon/mod.rs");

    // 1. The gain is applied to the decoded sample, before the EQ, so the
    //    bands and the reverb see the level it produced.
    assert!(
        decoder.contains("let sample = sample * self.pre_gain.amp();"),
        "the decode loop does not apply the pre-gain to the sample"
    );
    let at = decoder
        .find("let sample = sample * self.pre_gain.amp();")
        .expect("no pre-gain in the decode loop");
    let eq_at = decoder[at..]
        .find("// Apply EQ")
        .expect("no EQ after the pre-gain");
    assert!(
        eq_at < 400,
        "the pre-gain is applied after the EQ, so it is not a pre-gain"
    );
    // The right channel of a stereo pair is pulled separately and would skip the
    // gain entirely, which would pan a positive pre-gain hard left.
    assert!(
        decoder.contains("let right_raw = right_raw * self.pre_gain.amp();"),
        "the stereo right channel bypasses the pre-gain"
    );

    // 2. Every backend implements it, and the deferred one forwards rather than
    //    swallowing it — the deferred mixer is what the daemon actually holds.
    for (what, src) in [
        ("mixer", mixer),
        ("silent", include_str!("../src/audio/silent.rs")),
        ("pulse", include_str!("../src/audio/pulse.rs")),
    ] {
        assert!(
            src.contains("fn set_pre_gain(&self"),
            "{what} does not implement set_pre_gain"
        );
    }
    assert!(
        include_str!("../../gtmd/src/deferred_mixer.rs")
            .contains("fn set_pre_gain(&self, db: f32)"),
        "DeferredMixer does not forward set_pre_gain, so the daemon cannot use it"
    );

    // 3. The setter tells the mixer, not just the state file.
    let at = daemon
        .find("pub async fn set_pre_gain(")
        .expect("no set_pre_gain handler");
    let block = &daemon[at..at + 700];
    assert!(
        block.contains("set_pre_gain(pre_gain_db);"),
        "set_pre_gain writes state and emits an event but never reaches the mixer"
    );

    // 4. And the saved value is replayed into the mixer on first init, or it
    //    only takes effect once the user touches the control in this session.
    let at = daemon
        .find("let pre_gain_db = initial_state.audio.pre_gain_db;")
        .expect("pre_gain_db is not captured for the mixer factory");
    assert!(
        daemon[at..at + 900].contains("m.set_pre_gain(pre_gain_db);"),
        "the saved pre-gain is not replayed into the mixer at startup"
    );
}
