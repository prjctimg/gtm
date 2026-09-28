use crate::app::self_err;
use crate::app::*;

impl App {
    pub fn fetch_podcast_episodes(&mut self, feed_id: String) {
        self.podcast.episodes.clear();
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let fid = feed_id.clone();
        tokio::spawn(async move {
            match c.podcast().episodes(&fid).await {
                Ok((_title, eps)) => {
                    let _ = ipc_tx.send(IpcResult::PodcastEpisodes(eps));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("podcast episodes failed: {e}"));
                }
            }
        });
        self.podcast.episodes_feed_id = Some(feed_id);
    }

    /// Load the subscribed feed list, and the directory status alongside it.
    ///
    /// Shared by the podcast picker and the library pane's Podcasts category:
    /// both render the same `podcast.feeds`, so fetching lives here rather than
    /// in whichever caller happened to open first. Otherwise the pane would
    /// open to an empty list every time, since only the picker asked for the
    /// data.
    pub fn fetch_podcast_feeds(&mut self) {
        self.podcast.feeds_pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.podcast().feeds().await {
                Ok(f) => {
                    let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                    match c.podcast().status().await {
                        Ok(s) => {
                            let _ = ipc_tx.send(IpcResult::PodcastStatus(Some(s)));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("podcast status failed: {e}"));
                        }
                    }
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                }
            }
        });
    }
}
