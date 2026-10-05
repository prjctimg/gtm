use crate::app::self_err;
use crate::app::*;
use crate::shared::ipc::DaemonRes;

impl App {
    /// Re-pull the Top Charts source list (free providers always answer; the
    /// Spotify row appears/disappears with its link state).
    pub(crate) fn fetch_chart_sources(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.charts().sources().await {
                Ok(sources) => {
                    let _ = ipc_tx.send(IpcResult::ChartsSources(sources));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("chart sources failed: {e}"));
                }
            }
        });
    }

    /// Pull one of the three history lists for the Playlists view.
    ///
    /// Dispatched on [`HistList`], named rather than numbered. This used to match
    /// `7 => most_played, 8 => recently_played`, which were the *old* category
    /// indices — `Most Played` was category 5, not 7 — so every caller asked for
    /// the wrong list and only got away with it because the wrong answer was a
    /// list of tracks either way. The catch-all made it worse: a genuinely
    /// unknown category silently became "recently added". Nothing dispatches on
    /// a category index any more, so there is nothing left to fall through.
    pub(crate) fn fetch_list_tracks(&mut self, list: HistList) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let limit = 500u64;
        tokio::spawn(async move {
            let res = match list {
                HistList::Most => c.library().most_played(limit).await,
                HistList::Recent => c.library().recently_played(limit).await,
                HistList::Added => c.library().recently_added(limit).await,
            };
            let res = match res {
                Ok(DaemonRes::Tracks { tracks }) => match list {
                    HistList::Most => IpcResult::MostPlayed(*tracks),
                    HistList::Recent => IpcResult::RecentlyPlayed(*tracks),
                    HistList::Added => IpcResult::RecentlyAdded(*tracks),
                },
                Err(e) => IpcResult::Error(format!("failed to load list: {e}")),
                Ok(_) => return,
            };
            let _ = ipc_tx.send(res);
        });
    }

    /// Fetch the favourites, for the Playlists view's `Liked` group.
    pub(crate) fn fetch_favourites(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let res = match c.favourites().list().await {
                Ok(DaemonRes::Tracks { tracks }) => IpcResult::Favourites(*tracks),
                Err(e) => IpcResult::Error(format!("failed to load favourites: {e}")),
                Ok(_) => return,
            };
            let _ = ipc_tx.send(res);
        });
    }

    pub(crate) async fn fetch_queue(&mut self) {
        if let Ok(DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        }) = self.client.queue().list().await
        {
            self.queue.cache = *tracks;
            self.queue.cursor = cursor as usize;
        }
    }
}
