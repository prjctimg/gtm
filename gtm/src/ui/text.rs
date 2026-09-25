// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// User-facing string formatting
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub(crate) const INFO_CARD_H: u16 = 16;

pub(crate) const INFO_TEXT_H: u16 = 6;

pub(crate) fn info_block_h() -> u16 {
    if no_image_protocol() {
        INFO_TEXT_H
    } else {
        INFO_CARD_H
    }
}

pub(crate) fn library_stats_line(app: &App) -> String {
    if app.browse_detail.is_some() {
        if app.library_category == 5 {
            let n = app.spotify.playlist_tracks_cache.len();
            return format!(
                " {} {} (+ play all / shuffle) ",
                n,
                plural(n, "track", "tracks")
            );
        }
        let f = app.filtered_tracks();
        let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
        return format!(
            " {} {} | {}h {}m ",
            f.len(),
            plural(f.len(), "track", "tracks"),
            total_dur / 3600,
            (total_dur % 3600) / 60
        );
    }
    match app.library_category {
        2 => {
            let n = app.unique_albums().len();
            format!(" {} {} ", n, plural(n, "album", "albums"))
        }
        3 => {
            let n = app.unique_artists().len();
            format!(" {} {} ", n, plural(n, "artist", "artists"))
        }
        4 => {
            let n = app.playlist_cache.len();
            format!(" {} {} ", n, plural(n, "playlist", "playlists"))
        }
        5 => {
            let n = app.spotify.playlists.len();
            format!(" {} {} ", n, plural(n, "playlist", "playlists"))
        }
        _ => {
            let f = app.filtered_tracks();
            let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
            format!(
                " {} {} | {}h {}m | {} ",
                f.len(),
                plural(f.len(), "track", "tracks"),
                total_dur / 3600,
                (total_dur % 3600) / 60,
                app.track_sort.label()
            )
        }
    }
}

pub(crate) fn source_label(use_nerd: bool, source: &str) -> String {
    if use_nerd {
        match provider_icon(source) {
            Some(g) => format!(" {g} {source}"),
            None => " ♪ Local".to_string(),
        }
    } else {
        match source {
            "Spotify" => " ♫ Spotify",
            "YouTube" => " ▶ YouTube",
            "Local" => " ♪ Local",
            other => other,
        }
        .into()
    }
}

pub(crate) struct TrackInfoFields {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: Option<String>,
    pub(crate) meta: String,
    pub(crate) has_cover: bool,
}

pub(crate) fn track_info_fields(app: &App) -> Option<TrackInfoFields> {
    let use_nerd = use_nerd_fonts();
    match app.track_info_kind() {
        TrackInfoKind::Track => {
            let track = app
                .popup_track_id
                .and_then(|id| app.tracks_cache.iter().find(|t| t.id == id))?;
            let title = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                "Unknown".to_string()
            } else {
                track.artist.clone()
            };
            let album = if track.album.is_empty() {
                None
            } else {
                Some(track.album.clone())
            };
            let source = if track.path.contains("/audio/spotify")
                || track.path.starts_with("spotify:")
            {
                "Spotify"
            } else if track.path.contains("/audio/youtube") || track.path.starts_with("youtube:") {
                "YouTube"
            } else if let Some((src, _)) = classify_remote_source(&track.path) {
                src
            } else {
                "Local"
            };
            let meta = format!(
                " {} | {}",
                format_duration(track.duration as u64),
                source_label(use_nerd, source).trim_start()
            );
            let fav = if track.favourite { " \u{2665}" } else { "" };
            let meta = format!("{}{}", meta, fav);
            Some(TrackInfoFields {
                title,
                artist,
                album,
                meta,
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Album => {
            let albums = app.unique_albums();
            let pos = app.list_pos();
            let (name, count) = albums.get(pos)?;
            let artist = app
                .tracks_cache
                .iter()
                .find(|t| {
                    let album: &str = if t.album.is_empty() {
                        "Unknown Album"
                    } else {
                        &t.album
                    };
                    album == name
                })
                .map(|t| {
                    if t.artist.is_empty() {
                        "Unknown".to_string()
                    } else {
                        t.artist.clone()
                    }
                })
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: name.clone(),
                artist,
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    *count,
                    plural(*count, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Artist => {
            let artists = app.unique_artists();
            let pos = app.list_pos();
            let (name, count) = artists.get(pos)?;
            Some(TrackInfoFields {
                title: name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    *count,
                    plural(*count, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Playlist => {
            let playlists = &app.playlist_cache;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            let tc = pl.track_count as usize;
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    tc,
                    plural(tc, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyPlaylist => {
            let playlists = &app.spotify.playlists;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            let tc = pl.tracks.len();
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: pl.owner.clone(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    tc,
                    plural(tc, "track", "tracks"),
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyTrack => {
            let st = app.selected_spotify_track()?;
            let dur = st
                .duration_ms
                .map(|ms| format!(" [{}]", format_duration(ms / 1000)))
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: st.name.clone(),
                artist: st.artists.clone(),
                album: None,
                meta: format!(
                    "{} | {}",
                    dur,
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                has_cover: false,
            })
        }
    }
}

pub(crate) fn format_duration_short(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub(crate) fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}
