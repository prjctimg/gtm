use super::*;

use crate::spotify::track_art;

/// Cap on a fetched image. Station pages and tracklists are untrusted input and
/// nothing in the app needs more than a few hundred kilobytes of cover.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct Cover;

impl Cover {
    pub async fn get(
        inner: &DaemonInner,
        track_id: i64,
        path: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        inner.health.cover.count.fetch_add(1, Ordering::Relaxed);
        let track_path = path.as_deref();
        let mut discovered_artist = String::new();
        let mut discovered_album = String::new();

        // The SQLite library lookup runs on a blocking thread so a busy db
        // never stalls the command loop. The current cover art is picked
        // directly from the stored cover path or an audio sidecar.
        let library_track = if inner.config.test_mode {
            None
        } else {
            let data_dir = inner.config.data_dir.clone();
            tokio::task::spawn_blocking(move || {
                Library::new(data_dir.to_str().unwrap_or(""))
                    .ok()
                    .and_then(|lib| lib.get_track(track_id).ok().flatten())
            })
            .await
            .map_err(|e| CoreError::Daemon(e.to_string()))?
        };
        if let Some(ref track) = library_track
            && let Some(ref path) = track.cover_path
            && let Ok(data) = tokio::fs::read(path).await
            && !CoverCache::too_small(&data)
        {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            return Ok(DaemonRes::CoverArt { data: Some(b64) });
        }
        if let Some(ref track) = library_track {
            let audio_path = std::path::Path::new(&track.path);
            let parent = audio_path.parent().unwrap_or(std::path::Path::new(""));
            let stem = audio_path.file_stem().unwrap_or_default();
            for ext in ["jpg", "jpeg", "png", "webp"] {
                let sidecar = parent.join(format!("{}.{}", stem.to_string_lossy(), ext));
                if let Ok(data) = tokio::fs::read(&sidecar).await
                    && !CoverCache::too_small(&data)
                {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    return Ok(DaemonRes::CoverArt { data: Some(b64) });
                }
            }
            discovered_artist = track.artist.clone();
            discovered_album = track.album.clone();
        }

        // A queued entry can already know its artwork: the spotify resolver
        // points `cover_path` at the album art the web API handed us. Serve
        // that file before the artist/album search below, which misses often
        // enough to leave a playing track with no cover at all.
        let known_cover = {
            let state = inner.state.read().await;
            let by_key = if track_id == 0 {
                track_path.and_then(|p| {
                    state
                        .queue
                        .iter()
                        .chain(state.default_list.iter())
                        .find(|t| t.path == p)
                })
            } else {
                state
                    .queue
                    .iter()
                    .chain(state.default_list.iter())
                    .find(|t| t.id == track_id)
            };
            by_key.and_then(|t| t.cover_path.clone()).or_else(|| {
                state.current_track.as_ref().and_then(|t| {
                    let matches = t.id == track_id
                        || (track_id == 0 && track_path.is_some_and(|p| t.path == p));
                    matches.then(|| t.cover_path.clone()).flatten()
                })
            })
        };
        if let Some(path) = known_cover
            && let Ok(data) = tokio::fs::read(&path).await
            && !CoverCache::too_small(&data)
        {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            return Ok(DaemonRes::CoverArt { data: Some(b64) });
        }

        if discovered_artist.is_empty() {
            let state = inner.state.read().await;
            let mut in_merged = state.queue.iter().chain(state.default_list.iter());
            // Track ids of `0` collide across every locally-queued entry, so
            // resolve id-0 tracks by their exact path; otherwise match by id.
            let hit = if track_id == 0 {
                track_path.and_then(|p| in_merged.find(|t| t.path == p))
            } else {
                in_merged.find(|t| t.id == track_id)
            };
            if let Some(t) = hit {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            } else if let Some(ref t) = state.current_track
                && (t.id == track_id || (track_id == 0 && track_path.is_some_and(|p| t.path == p)))
            {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            }
        }

        // A live stream has no album art of its own. The track on air does
        // have a title, so resolve art from it, and fall back to the station's
        // own branding so the tile is never blank. Ordered cheapest-first:
        // art the tracklist already published, the track's own cover from the
        // metadata providers, then the station image.
        let radio_uuid = track_path
            .and_then(|p| p.strip_prefix("radio://"))
            .and_then(|rest| {
                let uuid = rest.split('/').next().unwrap_or(rest);
                (!uuid.is_empty()).then_some(uuid)
            });
        if let Some(uuid) = radio_uuid
            && let Some(bytes) = Self::radio_cover(inner, uuid).await
        {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(DaemonRes::CoverArt { data: Some(b64) });
        }

        if !discovered_artist.is_empty() && !discovered_album.is_empty() {
            let artist = discovered_artist.clone();
            let album = discovered_album.clone();
            let provider = inner.effective_cover_provider().await;

            // With Auto (the default) or an explicit Spotify preference, a
            // linked account supplies original 640x640 artwork ahead of the
            // network fallbacks below. Resolved with no lock held: taking the
            // cover cache here and the Spotify manager inside it is the ABBA
            // pair `Cover::artist` completes.
            if matches!(provider, CoverProvider::Auto | CoverProvider::Spotify)
                && let Ok(client) = linked(inner).await
                && let Some(bytes) = tokio::time::timeout(
                    Duration::from_secs(8),
                    album_cover(&client, &artist, &album),
                )
                .await
                .ok()
                .flatten()
            {
                let mut guard = inner.cover_cache().await;
                if let Some(ref mut cc) = *guard {
                    cc.put(&artist, &album, bytes.clone()).await;
                }
                drop(guard);
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Ok(DaemonRes::CoverArt { data: Some(b64) });
            }

            let mut guard = inner.cover_cache().await;
            if let Some(ref mut cache) = *guard {
                let cover = tokio::time::timeout(
                    Duration::from_secs(5),
                    cache.get(&artist, &album, provider),
                )
                .await
                .ok()
                .flatten();
                if let Some(cover) = cover {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                    return Ok(DaemonRes::CoverArt { data: Some(b64) });
                }
            }
        }

        Ok(DaemonRes::CoverArt { data: None })
    }

    /// Cover art for a live stream, in order of preference:
    ///
    /// 1. an image URL the station's tracklist published for the track;
    /// 2. the track's own cover, from Spotify when linked and otherwise from
    ///    the free-text Deezer search;
    /// 3. the station's Open Graph image from its homepage;
    /// 4. the station's directory favicon.
    ///
    /// Every step caches, so only the first call for a given track or station
    /// touches the network. All of them resolve to `None` rather than
    /// erroring, leaving the caller to render the placeholder glyph.
    pub(crate) async fn radio_cover(inner: &DaemonInner, uuid: &str) -> Option<Vec<u8>> {
        if let Some(bytes) = Self::track_cover(inner).await {
            return Some(bytes);
        }
        Self::station_cover(inner, uuid).await
    }

    /// The current live track's own cover, from the tracklist and the metadata
    /// providers. Reads the mirrored tracklist out of `state` rather than
    /// refetching, so a cached track costs no network at all.
    async fn track_cover(inner: &DaemonInner) -> Option<Vec<u8>> {
        let track = {
            let state = inner.state.read().await;
            let list = &state.radio_tracks;
            let at = match state.radio_title.as_deref() {
                Some(t) if !t.is_empty() => list.match_title(t),
                _ => list.at,
            };
            list.tracks.get(at)?.clone()
        };
        if let Some(url) = track.art.as_deref().filter(|u| !u.is_empty())
            && let Some(cover) = Self::url_cover(inner, url).await
        {
            return Some(cover);
        }
        let query = track.query();
        let provider = inner.effective_cover_provider().await;
        // Spotify first when linked: original artwork, and no rate limit.
        if matches!(provider, CoverProvider::Auto | CoverProvider::Spotify)
            && let Ok(client) = linked(inner).await
            && let Some(bytes) =
                tokio::time::timeout(Duration::from_secs(8), track_art(&client, &query))
                    .await
                    .ok()
                    .flatten()
        {
            let mut guard = inner.cover_cache().await;
            if let Some(cache) = guard.as_mut() {
                cache.put("track", &query, bytes.clone()).await;
            }
            return Some(bytes);
        }
        let mut guard = inner.cover_cache().await;
        let hit = match guard.as_mut() {
            Some(cache) => {
                tokio::time::timeout(Duration::from_secs(5), cache.get_text(&query, provider))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        hit.map(|c| c.data)
    }

    /// The station's own image, from its Open Graph tag or its directory
    /// favicon. Cached under the station uuid, and accepting images below the
    /// album cover floor since a station logo is legitimately small.
    async fn station_cover(inner: &DaemonInner, uuid: &str) -> Option<Vec<u8>> {
        {
            // Memory-then-disk only, so holding the cache lock across it is
            // safe; this is the same shape `Cover::artist` uses.
            let guard = inner.cover_cache().await;
            if let Some(hit) = guard.as_ref()
                && let Some(hit) = hit.get_station(uuid).await
            {
                return Some(hit.data);
            }
        }
        let (homepage, favicon) = {
            let radio = inner.radio.lock().await;
            radio
                .by_uuid(uuid)
                .await
                .map(|s| (s.homepage, s.favicon))
                .ok()?
        };
        let client = Self::image_client();
        if !homepage.is_empty()
            && let Some(og) = Self::og_image(&client, &homepage).await
        {
            let mut guard = inner.cover_cache().await;
            if let Some(cache) = guard.as_mut() {
                cache.put_station(uuid, og.clone()).await;
            }
            return Some(og);
        }
        let bytes = Self::fetch_image(&client, &favicon).await?;
        let mut guard = inner.cover_cache().await;
        if let Some(cache) = guard.as_mut() {
            cache.put_station(uuid, bytes.clone()).await;
        }
        Some(bytes)
    }

    /// Fetch an image URL through the cover cache, so a tracklist-advertised
    /// image reuses the same disk store as every other cover.
    async fn url_cover(inner: &DaemonInner, url: &str) -> Option<Vec<u8>> {
        let mut guard = inner.cover_cache().await;
        let cache = guard.as_mut()?;
        let client = Self::image_client();
        let hit = cache
            .get_url(url, || {
                let client = client.clone();
                let url = url.to_string();
                async move { Self::fetch_image(&client, &url).await }
            })
            .await?;
        Some(hit.data)
    }

    /// The `og:image` a station's homepage advertises, which is the closest
    /// thing it publishes to cover art. Tolerates either attribute order by
    /// locating `og:image` first and reading the `content` that follows.
    async fn og_image(client: &reqwest::Client, homepage: &str) -> Option<Vec<u8>> {
        let html = client.get(homepage).send().await.ok()?.text().await.ok()?;
        let at = html.find("og:image")?;
        let rest = &html[at + "og:image".len()..];
        let key = rest.find("content=")? + "content=".len();
        let rest = &rest[key..].trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let url = rest[1..].split(quote).next()?;
        Self::fetch_image(client, url).await
    }

    /// One bounded image GET. Station pages are untrusted input, so the body
    /// is capped and normalised before it reaches the cache.
    async fn fetch_image(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
        if url.trim().is_empty() {
            return None;
        }
        let resp = client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }
        Some(bytes.to_vec())
    }

    fn image_client() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(format!("gtm/{} ({})", env!("CARGO_PKG_VERSION"), "gtm"))
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    }

    pub async fn artist(inner: &DaemonInner, artist: &str) -> Result<DaemonRes, CoreError> {
        {
            let mut guard = inner.cover_cache().await;
            if let Some(ref mut cache) = *guard {
                let cover =
                    tokio::time::timeout(Duration::from_secs(8), cache.get_artist_image(artist))
                        .await
                        .ok()
                        .flatten();
                if let Some(cover) = cover {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                    return Ok(DaemonRes::CoverArt { data: Some(b64) });
                }
            }
        }

        // Spotify fallback for the artist portrait. Fetched before the cache is
        // retaken so the two locks are never held in the reverse order of
        // `Cover::track`.
        if let Ok(client) = linked(inner).await
            && let Some(bytes) =
                tokio::time::timeout(Duration::from_secs(8), artist_image(&client, artist))
                    .await
                    .ok()
                    .flatten()
        {
            {
                let mut guard = inner.cover_cache().await;
                if let Some(ref mut cache) = *guard {
                    cache.put_artist_image(artist, bytes.clone()).await;
                }
            }
            return Ok(DaemonRes::CoverArt {
                data: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            });
        }
        Ok(DaemonRes::CoverArt { data: None })
    }
}
