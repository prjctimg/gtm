use std::time::Duration;

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

    /// Search the public podcast directory.
    ///
    /// Separate from the subscription list on purpose: these are results to
    /// browse, and subscribing is a second step the user takes on the row they
    /// want. The daemon caches by query, so repeating a search is free.
    pub fn search_podcasts(&mut self, term: String) {
        let term = term.trim().to_string();
        // An empty term is allowed and means "what is popular". The daemon's
        // directory call answers it, and refusing it here is what left the
        // picker with nothing to show until the user had typed something.
        if term.is_empty() && !self.podcast.results.is_empty() {
            // Already holding the popular list: clearing and re-fetching on
            // every debounce tick would empty the pane the user is reading.
            return;
        }
        // A new term invalidates the old rows immediately: leaving yesterday's
        // results on screen under the new query is the one thing that would read
        // as a search that found them.
        self.podcast.results.clear();
        self.podcast.searching = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.podcast().discover(&term, "us").await {
                Ok(results) => {
                    let _ = ipc_tx.send(IpcResult::PodcastSearch(results));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::PodcastSearch(Vec::new()));
                    self_err(&ipc_tx, format!("podcast search failed: {e}"));
                }
            }
        });
    }

    /// Re-fetch every subscribed feed and reload the list.
    ///
    /// The refresh and the reload are two requests because the daemon answers
    /// the refresh with nothing: it returns a count, and the list has to be
    /// asked for again separately.
    pub fn refresh_podcast_feeds(&mut self, tx: &tokio::sync::mpsc::Sender<TuiCommand>) {
        let c = self.client.clone();
        let _ = tx.try_send(TuiCommand::fire(move || async move {
            let _ = c.podcast().refresh(None).await;
        }));
        self.podcast.feeds.clear();
        self.podcast.feeds_pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let _ = tx.try_send(TuiCommand::fire(move || async move {
            match c.podcast().feeds().await {
                Ok(f) => {
                    let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                }
            }
        }));
    }

    /// The rows the podcast picker is showing.
    ///
    /// A search replaces the subscriptions while it has results, which is the
    /// same switch [`render_podcast_feeds`](crate::ui::pickers::podcast::render_podcast_feeds)
    /// makes — the row count has to be the count of what is on screen or the
    /// cursor and the scroller address rows that are not there.
    /// Arm the directory search for after the debounce, and clear the rows the
    /// old query produced — a result list left under an edited query is the one
    /// state that reads as "the search found these".
    pub fn arm_podcast_search(&mut self) {
        // Not cleared unconditionally any more. An emptied query is the popular
        // list, so wiping the rows on every keystroke of the way back to empty
        // emptied the pane instead of returning it to the default view. The rows
        // go only when the new query is actually a search.
        if !self.picker_query_is_empty_podcast() {
            self.podcast.results.clear();
        }
        self.podcast.search_deadline =
            Some(std::time::Instant::now() + Duration::from_millis(SEARCH_DEBOUNCE_MS));
    }

    /// Whether the podcast picker's query is blank.
    fn picker_query_is_empty_podcast(&self) -> bool {
        self.pickers.top().is_some_and(|t| {
            t.id == crate::picker::PickerId::PodcastFeeds && t.query.trim().is_empty()
        })
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
