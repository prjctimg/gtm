// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify Web API: catalog search, track lookup, and playlist writes
//
// This is free software released under the GPL-3.0 license.

use rspotify::AuthCodePkceSpotify;
use rspotify::ClientError;
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::idtypes::Id;
use rspotify::model::{
    AlbumId, AlbumType, ArtistId, LibraryId, PlayableId, PlayableItem, PlaylistId, SearchType,
    SimplifiedArtist, Token, TrackId,
};
use tracing::warn;

use gtm::shared::spotify::{SpotifySearchKind, SpotifyTrack};

/// Per-request `limit` ceiling for `/v1/search` (10 since Spotify's February
/// 2026 migration, previously 50). Asking for more makes the request fail, so
/// the caller's budget is clamped to it.
const SEARCH_MAX: u32 = 10;

pub async fn access_token(client: &AuthCodePkceSpotify) -> Result<String, String> {
    match tokio::time::timeout(std::time::Duration::from_secs(10), client.auto_reauth()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(format!("spotify token refresh failed: {e}")),
        Err(_) => return Err("spotify token refresh timed out".to_string()),
    }
    let slot = client.get_token();
    let guard = slot
        .lock()
        .await
        .map_err(|_| "spotify token lock poisoned".to_string())?;
    let token: &Token = (*guard)
        .as_ref()
        .ok_or_else(|| "spotify token missing".to_string())?;
    Ok(token.access_token.clone())
}

/// Search the catalog for tracks, albums, artists and playlists.
///
/// [`SEARCH_MAX`] mirrors the per-request `limit` ceiling Spotify enforces
/// (10 since the February 2026 migration, down from 50); asking for more
/// makes the whole request fail, so the caller's budget is clamped here.
/// Each kind is queried independently and partial results are returned;
/// an error is only surfaced when every kind failed, so one bad query
/// cannot hide the other three.
pub async fn search(
    client: &AuthCodePkceSpotify,
    query: &str,
    limit: u32,
) -> Result<Vec<SpotifyTrack>, String> {
    let limit = limit.clamp(1, SEARCH_MAX);
    let sub = (limit / 2).max(3);
    let mut tracks: Vec<SpotifyTrack> = Vec::new();
    let mut errs: Vec<String> = Vec::new();

    // Track results first (the most useful).
    match client
        .search(query, SearchType::Track, None, None, Some(limit), None)
        .await
    {
        Ok(rspotify::model::SearchResult::Tracks(page)) => {
            tracks.extend(page.items.iter().enumerate().map(|(i, t)| SpotifyTrack {
                index: i,
                name: t.name.clone(),
                artists: artists_of(&t.artists),
                album: Some(t.album.name.clone()),
                duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
                uri: t.id.as_ref().map(|id| id.uri()),
                image_url: pick_largest_image(&t.album.images),
                kind: None,
            }));
        }
        Ok(_) => {}
        Err(e) => errs.push(format!("tracks: {e}")),
    }

    let mut idx = tracks.len();

    // Album results.
    match client
        .search(query, SearchType::Album, None, None, Some(sub), None)
        .await
    {
        Ok(rspotify::model::SearchResult::Albums(page)) => {
            for a in &page.items {
                tracks.push(SpotifyTrack {
                    index: idx,
                    name: a.name.clone(),
                    artists: artists_of(&a.artists),
                    album: Some(a.name.clone()),
                    duration_ms: None,
                    uri: a.id.as_ref().map(|id| id.uri()),
                    image_url: pick_largest_image(&a.images),
                    kind: Some(SpotifySearchKind::Album),
                });
                idx += 1;
            }
        }
        Ok(_) => {}
        Err(e) => errs.push(format!("albums: {e}")),
    }

    // Artist results.
    match client
        .search(query, SearchType::Artist, None, None, Some(sub), None)
        .await
    {
        Ok(rspotify::model::SearchResult::Artists(page)) => {
            for a in &page.items {
                tracks.push(SpotifyTrack {
                    index: idx,
                    name: a.name.clone(),
                    artists: String::new(),
                    album: None,
                    duration_ms: None,
                    uri: Some(a.id.uri()),
                    image_url: pick_largest_image(&a.images),
                    kind: Some(SpotifySearchKind::Artist),
                });
                idx += 1;
            }
        }
        Ok(_) => {}
        Err(e) => errs.push(format!("artists: {e}")),
    }

    // Playlist results. The owner display name rides in `artists` and the
    // track count in `album` so the TUI can render both without another
    // round-trip.
    match client
        .search(query, SearchType::Playlist, None, None, Some(sub), None)
        .await
    {
        Ok(rspotify::model::SearchResult::Playlists(page)) => {
            for a in &page.items {
                tracks.push(SpotifyTrack {
                    index: idx,
                    name: a.name.clone(),
                    artists: a.owner.display_name.clone().unwrap_or_default(),
                    album: Some(format!("{} tracks", a.items.total)),
                    duration_ms: None,
                    uri: Some(a.id.uri()),
                    image_url: pick_largest_image(&a.images),
                    kind: Some(SpotifySearchKind::Playlist),
                });
                idx += 1;
            }
        }
        Ok(_) => {}
        Err(e) => errs.push(format!("playlists: {e}")),
    }

    if tracks.is_empty() && !errs.is_empty() {
        return Err(errs.join("; "));
    }
    if !errs.is_empty() {
        warn!("spotify search partial failure: {}", errs.join("; "));
    }
    Ok(tracks)
}

/// Name a catalog endpoint Spotify has switched off, and say so.
///
/// Spotify's February 2026 Web API changes removed a batch of catalog
/// endpoints for Developer Mode integrations -- `artists`, `albums`,
/// `tracks`, `user`, `new_releases`, `categories` and `artist_top_tracks`
/// among them -- for new integrations from 11 Feb 2026 and for all existing
/// ones from 9 Mar 2026. See rspotify issue #550.
///
/// The three drill-down routes all sit on that list, so they now fail
/// permanently: `/albums/{id}/tracks` and `/playlists/{id}/tracks` answer 404
/// and `/artists/{id}/albums` answers 400. A bare "status code 400 Bad
/// Request" reads as a transient fault and invites a retry that can never
/// work, so name the cause instead. `/v1/search` and the playlist endpoints
/// are not on the list and still answer.
fn catalog_gone(what: &str, err: &ClientError) -> String {
    let text = err.to_string();
    // Only the two statuses Spotify returns for a switched-off endpoint; a 401
    // is a token problem and a 429 is a rate limit, and calling either of those
    // "removed" would send the reader down the wrong path.
    let removed = text.contains("404") || text.contains("400");
    if removed {
        format!(
            "{what}: {text} (Spotify removed this catalog endpoint for developer-mode \
             integrations in 2026; see rspotify issue #550)"
        )
    } else {
        format!("{what}: {text}")
    }
}

/// Resolve a web-search album result to its track list.
pub async fn album_tracks(
    client: &AuthCodePkceSpotify,
    uri: &str,
) -> Result<Vec<SpotifyTrack>, String> {
    let album_id = AlbumId::from_uri(uri).map_err(|e| format!("bad album uri: {e}"))?;
    let page = client
        .album_track_manual(album_id, None, Some(50), Some(0))
        .await
        .map_err(|e| catalog_gone("album tracks", &e))?;
    let mut tracks = Vec::new();
    for (i, t) in page.items.into_iter().enumerate() {
        tracks.push(SpotifyTrack {
            index: i,
            name: t.name.clone(),
            artists: artists_of(&t.artists),
            album: t.album.as_ref().map(|a| a.name.clone()),
            duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
            uri: t.id.as_ref().map(|id| id.uri()),
            image_url: t.album.as_ref().and_then(|a| pick_largest_image(&a.images)),
            kind: Some(SpotifySearchKind::Track),
        });
    }
    Ok(tracks)
}

/// Resolve an artist URI to their top tracks via their most recent albums.
/// Spotify removed the dedicated top-tracks endpoint, so we collect tracks
/// from the artist's newest albums/singles instead.
pub async fn artist_top(
    client: &AuthCodePkceSpotify,
    uri: &str,
) -> Result<Vec<SpotifyTrack>, String> {
    let artist_id = ArtistId::from_uri(uri).map_err(|e| format!("bad artist uri: {e}"))?;
    let page = client
        .artist_albums_manual(
            artist_id,
            [AlbumType::Album, AlbumType::Single],
            None,
            Some(20),
            Some(0),
        )
        .await
        .map_err(|e| catalog_gone("artist albums", &e))?;

    let mut tracks: Vec<SpotifyTrack> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target = 50u32;
    let mut albums_fetched = 0u32;

    for album in page.items {
        if tracks.len() as u32 >= target || albums_fetched >= 4 {
            break;
        }
        let Some(album_id) = album.id else {
            continue;
        };
        let page = match client
            .album_track_manual(album_id, None, Some(50), Some(0))
            .await
        {
            Ok(p) => p,
            Err(_) => continue,
        };
        albums_fetched += 1;
        for t in page.items {
            let Some(track_id) = t.id.as_ref() else {
                continue;
            };
            let track_uri = track_id.uri();
            if !seen.insert(track_uri.clone()) {
                continue;
            }
            tracks.push(SpotifyTrack {
                index: tracks.len(),
                name: t.name.clone(),
                artists: artists_of(&t.artists),
                album: t
                    .album
                    .as_ref()
                    .map(|a| a.name.clone())
                    .or(Some(album.name.clone())),
                duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
                uri: Some(track_uri),
                image_url: t.album.as_ref().and_then(|a| pick_largest_image(&a.images)),
                kind: Some(SpotifySearchKind::Track),
            });
            if tracks.len() as u32 >= target {
                break;
            }
        }
    }
    Ok(tracks)
}

/// Resolve a web-search playlist result (a `spotify:playlist:` URI) to its
/// track list so the TUI can queue and play it.
pub async fn web_playlist(
    client: &AuthCodePkceSpotify,
    uri: &str,
) -> Result<Vec<SpotifyTrack>, String> {
    let playlist_id = PlaylistId::from_uri(uri).map_err(|e| format!("bad playlist uri: {e}"))?;
    let page = client
        .playlist_items_manual(playlist_id, None, None, Some(50), Some(0))
        .await
        .map_err(|e| catalog_gone("playlist tracks", &e))?;
    let mut tracks = Vec::new();
    for item in page.items {
        if let Some(playable) = item.item.as_ref()
            && let Some(mut track) = track_from_playable(playable)
        {
            track.index = tracks.len();
            tracks.push(track);
        }
    }
    Ok(tracks)
}

/// Resolve a free-text `"{artist} - {title}"` query to a single Spotify track
/// URI, taking the closest hit. This is the bridge from a radio station's
/// published track name to a track Spotify can store, and it is also what the
/// cover lookup searches with.
pub async fn resolve_uri(client: &AuthCodePkceSpotify, query: &str) -> Result<String, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("empty track query".to_string());
    }
    let page = client
        .search(q, SearchType::Track, None, None, Some(5), None)
        .await
        .map_err(|e| format!("search: {e}"))?;
    let rspotify::model::SearchResult::Tracks(page) = page else {
        return Err("no track results".to_string());
    };
    page.items
        .iter()
        .find_map(|t| t.id.as_ref().map(|id| id.uri()))
        .ok_or_else(|| format!("no Spotify match for {q:?}"))
}

/// Full metadata for one track by bare id.
///
/// The queue holds nothing but a `spotify:track:<id>` path, and every label the
/// row needs — title, artists, album, duration, cover — lives here. Without
/// this the only thing a queued uri can be titled is `pretty_id`, which is the
/// literal "Spotify Track".
pub async fn track(client: &AuthCodePkceSpotify, id: &str) -> Option<SpotifyTrack> {
    let id = TrackId::from_id(id).ok()?;
    let full = client.track(id, None).await.ok()?;
    track_from_playable(&PlayableItem::Track(full))
}

/// Save a track to the user's Liked Songs. Needs the `user-library-modify`
/// scope, which a token minted before that scope existed cannot gain by
/// refreshing, so an older link must be re-authorized first.
pub async fn like(client: &AuthCodePkceSpotify, uri: &str) -> Result<(), String> {
    let id = TrackId::from_uri(uri).map_err(|e| format!("bad track uri: {e}"))?;
    client
        .library_add([LibraryId::Track(id)])
        .await
        .map_err(|e| format!("save to Liked Songs: {e}"))
}

/// Append a track to a playlist, the `POST /playlists/{id}/items` call. Needs
/// one of the `playlist-modify-*` scopes.
pub async fn playlist_add(
    client: &AuthCodePkceSpotify,
    playlist_id: &str,
    uri: &str,
) -> Result<(), String> {
    let list = PlaylistId::from_id(playlist_id)
        .or_else(|_| PlaylistId::from_uri(playlist_id))
        .map_err(|e| format!("bad playlist id: {e}"))?;
    let track = TrackId::from_uri(uri).map_err(|e| format!("bad track uri: {e}"))?;
    client
        .playlist_add_items(list, [PlayableId::Track(track)], None)
        .await
        .map(|_| ())
        .map_err(|e| format!("add to playlist: {e}"))
}

/// Comma-joined artist names of a track or album.
fn artists_of(artists: &[SimplifiedArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}
/// Pick the largest (first-sorted-by-area) image URL from a set of Spotify
/// image variants.
pub fn pick_largest_image(images: &[rspotify::model::Image]) -> Option<String> {
    images
        .iter()
        .max_by_key(|img| {
            let w = img.width.unwrap_or(0);
            let h = img.height.unwrap_or(0);
            w.saturating_mul(h)
        })
        .map(|img| img.url.clone())
}

/// Convert an rspotify playable item into our IPC-friendly track shape.
pub fn track_from_playable(item: &PlayableItem) -> Option<SpotifyTrack> {
    match item {
        PlayableItem::Track(t) => Some(SpotifyTrack {
            index: 0,
            name: t.name.clone(),
            artists: artists_of(&t.artists),
            album: Some(t.album.name.clone()),
            duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
            uri: t.id.as_ref().map(|id| id.uri()),
            image_url: pick_largest_image(&t.album.images),
            kind: None,
        }),
        PlayableItem::Episode(_) | PlayableItem::Unknown(_) => None,
    }
}
