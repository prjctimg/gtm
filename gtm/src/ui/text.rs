// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// User-facing string formatting
//
//
// This is free software released under the GPL-3.0 license.

use crate::app::{
    LIB_ALL, LIB_CHARTS, LIB_LIKED, LIB_PLAYLISTS, LIB_PODCASTS, LIB_RADIO, LIB_SPOTIFY,
    LibraryFilter, PlaylistGroup,
};
use crate::ui::*;

/// Most category rows the left pane will show at once.
///
/// The list is capped rather than given whatever the pane has left, because an
/// uncapped list takes every row and squeezes the cover art to nothing. Six
/// keeps the categories reachable on a short pane while leaving the card
/// usable on a tall one.
pub(crate) const LEFT_LIST_MAX_ROWS: u16 = 6;

/// Fewest rows the category list keeps when a preview card wants the space.
///
/// The card can shrink — its artwork is already sized from whatever box it is
/// handed, and the field block clips before that — while a category list that
/// drops to nothing takes the only route to every other category with it. Four
/// categories (All Tracks, Spotify, Radio, Top Charts) carry a preview from the
/// moment they are highlighted rather than only once something is loaded into
/// them, so they were the ones where an empty list appeared.
pub(crate) const LEFT_LIST_MIN_ROWS: u16 = 4;

/// Blank rows between the category list and the cover art below it.
///
/// Without it the two blocks abut, and the card's top border reads as another
/// row of the list rather than as a separate element.
pub(crate) const LEFT_LIST_PADDING: u16 = 1;

/// Extra blank rows above the info card, on top of [`LEFT_LIST_PADDING`].
///
/// The card is a full surface with its own art, and one row of clearance left
/// it visually attached to the list — long lists ran their last rows straight
/// into the artwork with nothing separating them.
pub(crate) const INFO_CARD_GAP: u16 = 1;

/// Rows to clear above the category list so its first row sits below the
/// now-playing cover image.
///
/// The two blocks belong to one visual unit: the cover is what the list is
/// browsing, so the list starts under it rather than level with its top. Three
/// rows lower than the cover's own offset — the cover is centred, so the
/// clearance is most of its slack, and the list's first rows were landing on the
/// artwork rather than beside it.
pub(crate) fn left_list_top(cover_band: u16) -> u16 {
    // The cover is centred in the band's inner rect (`cover.y = col.y +
    // (inner.height - cover_h) / 2`), and the band spends one row on its
    // header. Half the leftover slack, floored at the single row the header
    // already implies.
    cover_band.saturating_sub(1).max(4)
}

pub(crate) const INFO_CARD_H: u16 = 16;

pub(crate) const INFO_TEXT_H: u16 = 6;

/// Rows the track-info card's field block needs: title, artist, album, meta.
///
/// Fixed rather than per-track so the artwork above it does not change size
/// between a release with an album line and a single without one, and so the
/// floating card can size its box from the same count.
pub(crate) const INFO_FIELDS_H: u16 = 4;

pub(crate) fn info_block_h() -> u16 {
    if no_image_protocol() {
        INFO_TEXT_H
    } else {
        INFO_CARD_H
    }
}

pub(crate) fn library_stats_line(app: &App) -> String {
    if app.browse_detail.is_some() {
        if app.library_category == LIB_SPOTIFY {
            let n = app.spotify.playlist_tracks_cache.len();
            return format!(" {} {} ", n, plural(n, "track", "tracks"));
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
    // The grouped lists count their own rows. They used to fall through to the
    // track count below, so the footer read "812 tracks | 3h 12m" under a list
    // of forty genres — a count of the wrong list, sitting under the one thing
    // on screen that says how many there actually are.
    if app.library_category == LIB_ALL && !matches!(app.library_filter, LibraryFilter::Tracks) {
        let n = app.library_groups().len();
        return format!(
            " {} {} ",
            n,
            plural(n, app.library_filter.one(), app.library_filter.many())
        );
    }
    match app.library_category {
        LIB_PLAYLISTS => {
            // Each group counts its own rows: the playlists group counts
            // playlists, the three history groups count tracks. Reading
            // `playlist_cache` for all four left a rank-ordered list of tracks
            // under a count of playlists.
            if app.browse_detail.is_some() {
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
            let (n, one, many) = match app.playlist_group {
                PlaylistGroup::Playlists => (app.playlist_cache.len(), "playlist", "playlists"),
                PlaylistGroup::MostPlayed => (app.most_played_cache.len(), "track", "tracks"),
                PlaylistGroup::RecentlyPlayed => {
                    (app.recently_played_cache.len(), "track", "tracks")
                }
                PlaylistGroup::RecentlyAdded => (app.recently_added_cache.len(), "track", "tracks"),
            };
            format!(" {} {} ", n, plural(n, one, many))
        }
        LIB_SPOTIFY => {
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
            // The highlighted row of the list itself, not a lookup of
            // `popup_track_id` in `tracks_cache`.
            //
            // That lookup only works for a local library id, and it broke the
            // categories whose rows are not in `tracks_cache`: "All Tracks" is a
            // union of Spotify playlists whose rows are remote, so every one of
            // them carries `id == 0` and an empty path — the lookup matched
            // nothing (or, worse, an unrelated local track that happened to hold
            // id 0), so the card described a different song than the row under
            // the cursor. Reading `filtered_tracks()` at `list_pos()` is the same
            // source `update_track_popup` reads, so the two agree by
            // construction. `popup_track_id` stays as the fallback for the rows
            // that carry an id but sit outside the filtered list.
            let rows = app.filtered_tracks();
            let track: &TrackInfo = match rows.get(app.list_pos()) {
                Some(t) => t,
                None => app
                    .popup_track_id
                    .and_then(|id| app.tracks_cache.iter().find(|t| t.id == id))?,
            };
            let title = track.display_title();
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
        TrackInfoKind::ChartSource => {
            let src = app.charts.sources.get(app.list_pos())?;
            Some(TrackInfoFields {
                title: src.display.clone(),
                artist: "Charts".to_string(),
                album: None,
                // A source is a provider, not a chart: what the card can say is
                // whether it is usable, and which one it is.
                meta: format!(
                    "{} | {}",
                    if src.configured {
                        "configured"
                    } else {
                        "not linked"
                    },
                    src.id
                ),
                // Sources publish no artwork.
                has_cover: false,
            })
        }
        TrackInfoKind::Chart => {
            let ch = app.charts.charts.get(app.list_pos())?;
            let count = ch
                .track_count
                .map(|n| format!("{n} tracks"))
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: ch.title.clone(),
                artist: ch
                    .owner
                    .clone()
                    .filter(|o| !o.is_empty())
                    .unwrap_or_else(|| "Chart".to_string()),
                album: ch.description.clone().filter(|d| !d.is_empty()),
                meta: format!(
                    "{} | {}",
                    count,
                    source_label(use_nerd, chart_source_label(app)).trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::RadioStation => {
            let st = app.radio.custom.get(app.list_pos())?;
            // The host, not the whole stream URL: the path and query on a
            // station URL are long and identify nothing the reader can use.
            let host = st
                .url
                .split("://")
                .nth(1)
                .and_then(|r| r.split('/').next())
                .unwrap_or(st.url.as_str())
                .to_string();
            let mut meta = host;
            if st.uuid.is_some() {
                meta.push_str(" | directory");
            }
            if st.tracklist.is_some() {
                meta.push_str(" | tracklist");
            }
            Some(TrackInfoFields {
                title: st.name.clone(),
                artist: "Radio".to_string(),
                album: None,
                meta,
                // See `App::popup_cover_url`: a custom station has no icon
                // without a directory lookup.
                has_cover: false,
            })
        }
        TrackInfoKind::ChartTrack => {
            let ct = app.charts.chart_tracks.get(app.list_pos())?;
            let dur = ct
                .duration_ms
                .map(|ms| format!(" [{}]", format_duration(ms / 1000)))
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: ct.title.clone(),
                artist: if ct.artists.is_empty() {
                    "Unknown".to_string()
                } else {
                    ct.artists.clone()
                },
                album: ct.album.clone(),
                // A chart row has no path to classify, so the source is the
                // chart provider the row came from.
                meta: format!(
                    "{} | {}",
                    dur,
                    source_label(use_nerd, chart_source_label(app)).trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
    }
}

/// Which chart provider the loaded chart came from, for the info card.
fn chart_source_label(app: &App) -> &'static str {
    app.charts
        .selected_source
        .and_then(|i| app.charts.sources.get(i))
        .map_or("Chart", |s| match s.id.as_str() {
            "spotify" => "Spotify",
            "apple" => "Apple Music",
            _ => "Chart",
        })
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
