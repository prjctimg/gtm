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
    ) -> Result<DaemonRes, CoreError> {
        if let Some(manager) = inner.lyrics_manager().await {
            let lyrics =
                tokio::time::timeout(Duration::from_secs(10), manager.search(artist, title))
                    .await
                    .ok()
                    .flatten();
            Ok(DaemonRes::Lyrics { lyrics })
        } else {
            Ok(DaemonRes::Lyrics { lyrics: None })
        }
    }
}
