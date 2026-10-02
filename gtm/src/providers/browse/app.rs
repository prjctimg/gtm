use std::time::Duration;

use crate::app::self_err;
use crate::app::*;
use crate::shared::chart::BrowseHit;

/// Music browse: search, then drill into an album's tracklist or an artist's
/// page.
///
/// The three levels share one list rather than three surfaces because they
/// answer one question in sequence: what is this, and what else is there. A
/// row is played, opened, or backed out of, and only the level changes.
///
/// The provider here is Deezer, and the reason is that Spotify cannot answer
/// any of it: its artist-contents endpoints were removed for developer-mode
/// integrations, so there is no request to make for an artist's albums or an
/// album's tracklist. Deezer's public API needs no key.
impl App {
    /// Arm the search for after the debounce, and drop the previous results.
    ///
    /// The rows go immediately rather than when the new ones arrive: leaving the
    /// old results under an edited query is the one state that reads as "these
    /// are what you asked for".
    pub fn arm_browse(&mut self) {
        self.browse.hits.clear();
        self.browse.rows.clear();
        self.browse.releases.clear();
        self.browse.album = None;
        self.browse.artist = None;
        self.browse.level = BrowseLevel::Results;
        self.browse.deadline =
            Some(std::time::Instant::now() + Duration::from_millis(SEARCH_DEBOUNCE_MS));
    }

    /// Send the picker's query to the provider.
    pub fn browse_search(&mut self, term: String) {
        let term = term.trim().to_string();
        if term.is_empty() {
            self.browse.hits.clear();
            self.browse.pending = false;
            return;
        }
        self.browse.pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.browse().search(&term).await {
                Ok(hits) => {
                    let _ = ipc_tx.send(IpcResult::BrowseHits(hits));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::BrowseHits(Vec::new()));
                    self_err(&ipc_tx, format!("browse search failed: {e}"));
                }
            }
        });
    }

    /// Open an album's tracklist.
    pub fn browse_album(&mut self, album_id: u64) {
        self.browse.pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.browse().album(album_id).await {
                Ok(page) => {
                    let _ = ipc_tx.send(IpcResult::BrowseAlbumPage(page));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("browse album failed: {e}"));
                }
            }
        });
    }

    /// Open an artist's page.
    pub fn browse_artist(&mut self, artist_id: u64) {
        self.browse.pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.browse().artist(artist_id).await {
                Ok(page) => {
                    let _ = ipc_tx.send(IpcResult::BrowseArtistPage(page));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("browse artist failed: {e}"));
                }
            }
        });
    }

    /// Back out to the search results from either drill-down.
    pub fn browse_up(&mut self) {
        self.browse.level = BrowseLevel::Results;
        self.browse.album = None;
        self.browse.artist = None;
        self.browse.rows.clear();
        self.browse.releases.clear();
        self.browse.heading.clear();
        self.browse.pending = false;
        if let Some(top) = self.pickers.top_mut() {
            top.selected = 0;
            top.viewport_offset = 0;
        }
        self.data_dirty = true;
    }

    /// Number of rows the Browse pane draws at its current level.
    pub fn browse_len(&self) -> usize {
        match self.browse.level {
            // Search results are hits, which are three kinds — not tracks.
            BrowseLevel::Results => self.browse.hits.len(),
            BrowseLevel::Album => self.browse.rows.len(),
            // An artist page is its top tracks followed by its releases, which
            // are rendered as one list with a header between them.
            BrowseLevel::Artist => self.browse.rows.len() + self.browse.releases.len(),
        }
    }

    /// Act on Enter for the highlighted Browse row.
    ///
    /// A song plays. A release or a person opens: an album's own tracklist is
    /// the answer a metadata lookup exists to give, and a person's page is where
    /// the rest of their work is.
    pub(crate) async fn browse_enter(
        &mut self,
        ipc_tx: &tokio::sync::mpsc::UnboundedSender<IpcResult>,
    ) {
        let sel = self.list_pos();
        match self.browse.level {
            BrowseLevel::Results => match self.browse.hits.get(sel).cloned() {
                Some(BrowseHit::Track(t)) => {
                    self.play_browse_track(
                        &t.title,
                        &t.artist,
                        t.album.as_deref(),
                        t.preview_url,
                        ipc_tx,
                    )
                    .await;
                }
                Some(BrowseHit::Album(a)) => {
                    self.browse.heading = format!("{} \u{2014} {}", a.title, a.artist);
                    self.browse_album(a.id);
                }
                Some(BrowseHit::Artist(a)) => {
                    self.browse.heading = a.name.clone();
                    self.browse_artist(a.id);
                }
                None => {}
            },
            BrowseLevel::Album | BrowseLevel::Artist => {
                if let Some(t) = self.browse.rows.get(sel).cloned() {
                    self.play_browse_track(
                        &t.title,
                        &t.artist,
                        t.album.as_deref(),
                        t.preview_url,
                        ipc_tx,
                    )
                    .await;
                }
            }
        }
    }

    /// Play one browsed song.
    ///
    /// Routed through the Spotify resolve path, exactly as a chart row is: the
    /// provider here only publishes a 30-second preview, and the point of the
    /// browse tree is the full track. The row's own metadata travels with the
    /// request so the queue shows what the user picked rather than whatever
    /// the match happened to be.
    async fn play_browse_track(
        &mut self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        preview: Option<String>,
        ipc_tx: &tokio::sync::mpsc::UnboundedSender<IpcResult>,
    ) {
        let (title, artist, album, preview) = (
            title.to_string(),
            artist.to_string(),
            album.map(str::to_string),
            preview,
        );
        let c = self.client.clone();
        let ipc = ipc_tx.clone();
        tokio::spawn(async move {
            let query = if artist.trim().is_empty() {
                title.clone()
            } else {
                format!("{artist} - {title}")
            };
            let uri = c.spotify().match_track(&query).await.ok();
            if let Err(e) = c
                .spotify()
                .resolve_track(
                    &title,
                    &artist,
                    album.as_deref().unwrap_or(""),
                    uri,
                    None,
                    true,
                )
                .await
            {
                // No Spotify account, or nothing matched. The preview is a real
                // 30 seconds of the song, so it plays rather than leaving a row
                // that silently does nothing.
                match preview {
                    Some(url) => {
                        let _ = c.play_stream(&url).await;
                    }
                    None => {
                        let _ = ipc.send(IpcResult::Error(format!("{title}\u{2014}{e}")));
                    }
                }
            }
        });
    }
}
