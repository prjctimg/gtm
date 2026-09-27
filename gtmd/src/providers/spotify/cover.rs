// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify artwork: album, track, and artist image lookups over the Web API
//
// This is free software released under the GPL-3.0 license.

use rspotify::AuthCodePkceSpotify;
use rspotify::clients::BaseClient;
use rspotify::model::SearchType;

use super::api::{access_token, pick_largest_image};

/// Fetch the largest album-cover image bytes for an artist + album via the
/// Web API. `None` when no hit.
pub async fn album_cover(
    client: &AuthCodePkceSpotify,
    artist: &str,
    album: &str,
) -> Option<Vec<u8>> {
    let q = format!("album:\"{}\" artist:\"{}\"", album.trim(), artist.trim());
    let page = client
        .search(&q, SearchType::Album, None, None, Some(3), None)
        .await
        .ok()?;
    let rspotify::model::SearchResult::Albums(page) = page else {
        return None;
    };
    let images = page
        .items
        .into_iter()
        .flat_map(|a| a.images)
        .collect::<Vec<_>>();
    let url = pick_largest_image(&images)?;
    fetch_image(client, &url).await
}

/// Fetch the largest album-cover image bytes for a free-text `"{artist} -
/// {title}"` query via the Web API. This is the shape a radio station
/// publishes, and free text matches where the `album:` / `artist:` field
/// syntax misses a track released outside Spotify's album index. `None` when
/// no hit.
pub async fn track_art(client: &AuthCodePkceSpotify, query: &str) -> Option<Vec<u8>> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let page = client
        .search(q, SearchType::Track, None, None, Some(3), None)
        .await
        .ok()?;
    let rspotify::model::SearchResult::Tracks(page) = page else {
        return None;
    };
    let images = page
        .items
        .into_iter()
        .flat_map(|t| t.album.images)
        .collect::<Vec<_>>();
    let url = pick_largest_image(&images)?;
    fetch_image(client, &url).await
}

/// Fetch the largest artist portrait image bytes via the Web API. `None` when
/// no hit.
pub async fn artist_image(client: &AuthCodePkceSpotify, artist: &str) -> Option<Vec<u8>> {
    let q = format!("artist:\"{}\"", artist.trim());
    let page = client
        .search(&q, SearchType::Artist, None, None, Some(3), None)
        .await
        .ok()?;
    let rspotify::model::SearchResult::Artists(page) = page else {
        return None;
    };
    let images = page
        .items
        .into_iter()
        .flat_map(|a| a.images)
        .collect::<Vec<_>>();
    let url = pick_largest_image(&images)?;
    fetch_image(client, &url).await
}

/// Fetch the raw bytes of an album-cover image located at `url` (as exposed via
/// [`SpotifyTrack::image_url`]), without an extra search.
pub async fn image_at(client: &AuthCodePkceSpotify, url: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    fetch_image(client, url).await
}

async fn fetch_image(client: &AuthCodePkceSpotify, url: &str) -> Option<Vec<u8>> {
    let token = access_token(client).await.ok()?;
    let resp = reqwest::Client::new()
        .get(url)
        .bearer_auth(&token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (!bytes.is_empty()).then_some(bytes.to_vec())
}
