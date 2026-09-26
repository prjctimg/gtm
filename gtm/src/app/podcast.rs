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
}
