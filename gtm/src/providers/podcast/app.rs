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
        let directory = term.is_empty();
        // Already holding the popular list: clearing and re-fetching on every
        // debounce tick would empty the pane the user is reading.
        if directory && self.podcast.directory && !self.podcast.results.is_empty() {
            return;
        }
        // A new term invalidates the old rows immediately: leaving yesterday's
        // results on screen under the new query is the one thing that would read
        // as a search that found them.
        self.podcast.results.clear();
        self.podcast.directory = directory;
        self.podcast.searching = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // The empty term is the picker's opening window: "what is popular
            // now". That is an ranked phrase — Apple publishes a list — so it
            // asks the charts endpoint, which resolves the top-25 to
            // subscribable feeds, rather than the directory search whose
            // no-term page is relevance-ranked and drifts with iTunes' own
            // idea of an ordering.
            let res = if directory {
                c.podcast().charts("us", "").await
            } else {
                c.podcast().discover(&term, "us").await
            };
            match res {
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
        // The rows go only when the new query is actually a search. An emptied
        // query is the directory's own landing page, and `search_podcasts`
        // already leaves an already-shown landing page alone — so wiping here
        // would empty the pane on every keystroke of the way back to blank
        // instead of letting it settle back to the directory.
        if !self.picker_query_is_empty_podcast() {
            self.podcast.results.clear();
            self.podcast.directory = false;
        }
        self.podcast.search_deadline =
            Some(std::time::Instant::now() + Duration::from_millis(SEARCH_DEBOUNCE_MS));
    }

    /// Open the picker on the directory rather than on the user's own
    /// subscriptions.
    ///
    /// The category's first screen used to be whatever the user had already
    /// subscribed to, so a fresh install opened a picker with nothing in it and
    /// the only way out was knowing that `/` searches. The Radio category
    /// reaches Radio Browser the same way, and the directory — what is popular
    /// — is the answer to "what can I listen to" for someone who has not
    /// subscribed to anything yet.
    pub fn open_podcast_dir(&mut self) {
        self.pickers.open(PickerId::PodcastFeeds);
        self.arm_podcast_search();
    }

    /// Open the URL form for subscribing to a feed.
    ///
    /// One entry point because both the feed picker and the library category's
    /// `a` reach it, and the form's draft lives on `podcast` rather than in the
    /// feed picker's query.
    pub fn open_podcast_subscribe(&mut self) {
        self.podcast.subscribe_url.clear();
        self.pickers.open(PickerId::PodcastSubscribe);
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
