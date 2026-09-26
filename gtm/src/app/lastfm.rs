use crate::app::*;

impl App {
    /// Re-pull the Last.fm link status into the setup view.
    pub fn refresh_lastfm_status(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.lastfm().status().await {
                Ok(st) => {
                    let _ = ipc_tx.send(IpcResult::LastfmStatus(Some(st)));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("last.fm status failed: {e}"));
                }
            }
        });
    }

    /// Love or un-love the current track on Last.fm. `*` toggles based on the
    /// last known loved state; the daemon immediate-scrobbles on love.
    pub(crate) fn manage_lastfm_love(&mut self) -> bool {
        if self.library_pane_focus {
            return true;
        }
        let love = !self.setup.lastfm_status.as_ref().is_some_and(|s| s.loved);
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let result = if love {
                c.lastfm().love().await
            } else {
                c.lastfm().unlove().await
            };
            match result {
                Ok(()) => {}
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Last.fm love failed: {e}")));
                }
            }
        });
        self.refresh_lastfm_status();
        false
    }

    /// Flip Last.fm scrobbling on/off for this session (`&`).
    pub(crate) fn toggle_scrobble_session(&mut self) {
        let enabled = !self.setup.lastfm_status.as_ref().is_some_and(|s| s.enabled);
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c
                .lastfm()
                .set_config(enabled, None, None, None, None, None)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Last.fm toggle failed: {e}")));
                }
            }
        });
        self.refresh_lastfm_status();
    }
}
