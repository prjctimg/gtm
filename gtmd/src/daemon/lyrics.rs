use super::*;

pub(crate) struct Lyrics;

impl Lyrics {
    pub async fn get(
        inner: &DaemonInner,
        track_id: i64,
        path: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        inner.health.lyrics.count.fetch_add(1, Ordering::Relaxed);
        let current = {
            let state = inner.state.read().await;
            state.current_track.as_ref().and_then(|t| {
                let id_matches = t.id == track_id;
                let path_matches = path.as_deref().is_some_and(|p| t.path == p);
                if id_matches || path_matches {
                    Some(t.clone())
                } else {
                    None
                }
            })
        };
        let track = match current {
            Some(t) => t,
            None => {
                let resolved = if !inner.config.test_mode {
                    let data_dir = inner.config.data_dir.clone();
                    tokio::task::spawn_blocking(move || {
                        Library::new(data_dir.to_str().unwrap_or(""))
                            .ok()
                            .and_then(|lib| lib.get_track(track_id).ok().flatten())
                    })
                    .await
                    .map_err(|e| CoreError::Daemon(e.to_string()))?
                } else {
                    None
                };
                match resolved.or_else(|| path.map(|p| queue::resolve_track(&p))) {
                    Some(t) => t,
                    None => return Ok(DaemonRes::Lyrics { lyrics: None }),
                }
            }
        };

        // A provider entry already carries the artist and title the playlist
        // sync cached, so it is worth searching directly. The library route
        // below resolves a track by id, which a `spotify:` entry has none of,
        // and that is why a Spotify track's lyrics came back empty.
        if track.path.starts_with("spotify:") {
            return Ok(crate::providers::spotify::lyrics::for_track(inner, &track).await);
        }

        let mut track = track;
        if track.artist.is_empty() || track.title.is_empty() {
            let (artist, title) = meta_from_filename(&track.path);
            if track.artist.is_empty() {
                track.artist = artist;
            }
            if track.title.is_empty() {
                track.title = title;
            }
        }

        if let Some(manager) = inner.lyrics_manager().await {
            let lyrics = tokio::time::timeout(Duration::from_secs(10), manager.get_lyrics(&track))
                .await
                .ok()
                .flatten();
            Ok(DaemonRes::Lyrics { lyrics })
        } else {
            Ok(DaemonRes::Lyrics { lyrics: None })
        }
    }

    pub async fn search(
        inner: &DaemonInner,
        artist: &str,
        title: &str,
        album: Option<&str>,
        duration: Option<f64>,
    ) -> Result<DaemonRes, CoreError> {
        if let Some(manager) = inner.lyrics_manager().await {
            // With an album and duration the caller holds everything lrclib's
            // exact `/api/get` needs, so build a track and take the same strong
            // path a library file does — disk cache first, then exact, then the
            // looser fallbacks. Without them, fall back to the loose search the
            // two-field request has always used.
            let lyrics = if album.is_some() || duration.is_some() {
                let track = TrackInfo {
                    title: title.to_string(),
                    artist: artist.to_string(),
                    album: album.unwrap_or_default().to_string(),
                    duration: duration.unwrap_or_default(),
                    ..Default::default()
                };
                tokio::time::timeout(Duration::from_secs(10), manager.get_lyrics(&track))
                    .await
                    .ok()
                    .flatten()
            } else {
                tokio::time::timeout(Duration::from_secs(10), manager.search(artist, title))
                    .await
                    .ok()
                    .flatten()
            };
            Ok(DaemonRes::Lyrics { lyrics })
        } else {
            Ok(DaemonRes::Lyrics { lyrics: None })
        }
    }
}
