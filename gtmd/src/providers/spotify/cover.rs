// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify artwork: album, track, and artist image lookups over the Web API
//
// This is free software released under the GPL-3.0 license.

use futures::future::join_all;
use rspotify::AuthCodePkceSpotify;
use rspotify::clients::BaseClient;
use rspotify::model::SearchType;
use tracing::{debug, info};

use gtm::shared::spotify::SpotifyTrack;
use gtm::shared::track::TrackInfo;

use crate::daemon::DaemonInner;

use super::api::{access_token, pick_largest_image};
use super::cmd::linked;

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

/// How many covers to fetch at once. Spotify's CDN tolerates more, but the
/// daemon also serves the TUI from these caches, and a thousand-track playlist
/// opening at full concurrency starves the track that is actually playing.
const PREFETCH_CONCURRENCY: usize = 8;

/// Warm the cover cache for every distinct album image in `tracks`.
///
/// Browsing a playlist used to fetch each cover on demand, one per scroll or
/// Enter, and a cover already seen was only remembered in the 64-entry memory
/// LRU. Persisting through [`CoverCache::get_url`] keys each image by URL on
/// disk, so the second visit — and the next login, which throws the in-memory
/// half away — is served without a request.
///
/// Runs detached and never surfaces an error: a missing cover is a blank row,
/// not a failure worth interrupting a sync for.
pub(crate) async fn prefetch(inner: &DaemonInner, tracks: &[SpotifyTrack]) {
    let mut urls: Vec<String> = Vec::new();
    for t in tracks {
        if let Some(url) = t.image_url.as_deref().filter(|u| !u.trim().is_empty())
            && !urls.iter().any(|u| u == url)
        {
            urls.push(url.to_string());
        }
    }
    if urls.is_empty() {
        return;
    }
    let total = urls.len();
    info!("spotify: prefetching {total} playlist covers");
    let Ok(client) = linked(inner).await else {
        return;
    };
    for chunk in urls.chunks(PREFETCH_CONCURRENCY) {
        // Polled together rather than spawned: the futures borrow `inner`, and
        // the cover cache is a mutex, so bounding the group is what bounds the
        // concurrency.
        let warm = chunk.iter().map(|url| {
            let client = client.clone();
            async move {
                let mut guard = inner.cover_cache().await;
                let Some(cache) = guard.as_mut() else {
                    return false;
                };
                cache
                    .get_url(url, || image_at(&client, url))
                    .await
                    .is_some()
            }
        });
        for hit in join_all(warm).await {
            if !hit {
                debug!("spotify: a prefetched cover was unavailable");
            }
        }
    }
}

/// How far ahead of the end of a track its successor's artwork is warmed.
pub(crate) const PRELOAD_LEAD: f64 = 15.0;

/// Warm the artwork for one track that is about to start.
///
/// The single-track companion to [`prefetch`], driven by the position tick. A
/// cover looked up when a track reaches the top of the queue is a request the
/// user waits through; warming it while the current track is still playing puts
/// the bytes on disk first, so the advance paints with artwork already there.
///
/// The entry's `cover_url` is used when the provider supplied one and the
/// artist/album search otherwise. Returns whether anything was warmed, so the
/// caller can latch on it and ask only once per track.
pub(crate) async fn preload(inner: &DaemonInner, track: &TrackInfo) -> bool {
    if track.cover_path.as_deref().is_some_and(|p| !p.is_empty()) {
        return false;
    }
    let url = track.cover_url.clone().filter(|u| !u.trim().is_empty());
    let client = match url {
        Some(_) => linked(inner).await.ok(),
        None => None,
    };
    let mut guard = inner.cover_cache().await;
    let Some(cache) = guard.as_mut() else {
        return false;
    };
    let hit = match (url.as_deref(), client.as_ref()) {
        (Some(u), Some(c)) => cache.get_url(u, || image_at(c, u)).await,
        _ => {
            if track.artist.is_empty() || track.album.is_empty() {
                return false;
            }
            cache
                .get(
                    &track.artist,
                    &track.album,
                    inner.effective_cover_provider().await,
                )
                .await
        }
    };
    hit.is_some()
}
