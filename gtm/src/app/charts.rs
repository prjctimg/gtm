use crate::app::*;

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

    pub(crate) fn fetch_list_tracks(&mut self, category: usize) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let limit = 500u64;
        tokio::spawn(async move {
            let res = match category {
                7 => c.library().most_played(limit).await,
                8 => c.library().recently_played(limit).await,
                _ => c.library().recently_added(limit).await,
            };
            let res = match res {
                Ok(DaemonRes::Tracks { tracks }) => match category {
                    7 => IpcResult::MostPlayed(*tracks),
                    8 => IpcResult::RecentlyPlayed(*tracks),
                    _ => IpcResult::RecentlyAdded(*tracks),
                },
                Err(e) => IpcResult::Error(format!("failed to load list: {e}")),
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
