use super::*;

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

        // Radio stations have no album art; serve the station's own favicon
        // (fetched once, cached under `radio:<uuid>`) so the now-playing pane
        // shows the station logo instead of a blank tile.
        let radio_uuid = track_path
            .and_then(|p| p.strip_prefix("radio://"))
            .and_then(|rest| {
                let uuid = rest.split('/').next().unwrap_or(rest);
                (!uuid.is_empty()).then_some(uuid)
            });
        if let Some(uuid) = radio_uuid
            && let Some(bytes) = Self::radio_favicon_cover(inner, uuid).await
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

    /// Resolve a radio station's favicon to cover-sized bytes, using the
    /// directory's `byuuid` lookup to find the station (and thus its favicon
    /// URL), then fetching the PNG once and caching it under `radio:<uuid>`.
    pub(crate) async fn radio_favicon_cover(inner: &DaemonInner, uuid: &str) -> Option<Vec<u8>> {
        {
            let mut guard = inner.cover_cache().await;
            let hit = match guard.as_mut() {
                Some(c) => c.get("radio", uuid, CoverProvider::Auto).await,
                None => None,
            };
            if let Some(cover) = hit {
                return Some(cover.data);
            }
        }
        let favicon = {
            let radio = inner.radio.lock().await;
            radio.by_uuid(uuid).await.map(|s| s.favicon).ok()
        }
        .into_iter()
        .find(|f| !f.is_empty())?;
        let bytes = tokio::time::timeout(Duration::from_secs(8), async {
            let req = reqwest::Client::builder()
                .user_agent(format!("gtm/{}", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(8))
                .build()
                .ok()?;
            req.get(&favicon)
                .send()
                .await
                .ok()?
                .bytes()
                .await
                .ok()
                .map(|b| b.to_vec())
        })
        .await
        .ok()??;
        if CoverCache::too_small(&bytes) {
            return None;
        }
        let mut guard = inner.cover_cache().await;
        if let Some(cache) = guard.as_mut() {
            cache.put("radio", uuid, bytes.clone()).await;
        }
        Some(bytes)
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
