use super::*;

pub(crate) struct Lastfm;

impl Lastfm {
    pub async fn set_config(
        inner: &DaemonInner,
        enabled: bool,
        api_key: Option<String>,
        api_secret: Option<String>,
        session_key: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<DaemonRes, CoreError> {
        // Persist any new credentials in the OS keychain so they survive daemon
        // restarts; blank values mean "keep what is already stored".
        if let Some(key) = api_key.as_ref().filter(|k| !k.trim().is_empty()) {
            set_secret(LASTFM_API_KEY, key.trim());
        }
        if let Some(secret) = api_secret.as_ref().filter(|s| !s.trim().is_empty()) {
            set_secret(LASTFM_API_SECRET, secret.trim());
        }

        let current_session = inner.state.read().await.scrobble.session_token.clone();

        let effective_session = session_key
            .filter(|s| !s.trim().is_empty())
            .or(current_session);
        let effective_key = if enabled {
            api_key
                .filter(|k| !k.trim().is_empty())
                .or_else(|| get_secret(LASTFM_API_KEY))
        } else {
            None
        };
        let effective_secret = if enabled {
            api_secret
                .filter(|s| !s.trim().is_empty())
                .or_else(|| get_secret(LASTFM_API_SECRET))
        } else {
            None
        };
        if let (Some(key), Some(secret)) = (&effective_key, &effective_secret) {
            let mut lastfm = inner.lastfm.lock().await;
            lastfm
                .init(key.clone(), secret.clone(), effective_session.clone())
                .await;
        }

        let mut state = inner.state.write().await;
        state.set_scrobble(
            enabled,
            effective_key,
            effective_session,
            min_play_secs,
            min_play_pct,
        )?;
        drop(state);
        Daemon::push_event(inner, DaemonEvent::ScrobbleConfigChanged { enabled });
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Re-initialize the Last.fm manager from the stored keyring credentials
    /// and the saved session token, so scrobbling survives a daemon restart
    /// without re-running the setup wizard.
    pub async fn restore_credentials(inner: &DaemonInner) {
        let api_key = get_secret(LASTFM_API_KEY);
        let api_secret = get_secret(LASTFM_API_SECRET);
        let session = inner.state.read().await.scrobble.session_token.clone();
        if let (Some(api_key), Some(api_secret)) = (api_key, api_secret) {
            let mut lastfm = inner.lastfm.lock().await;
            lastfm.init(api_key, api_secret, session).await;
        }
    }

    /// Point the manager at the on-disk retry queue and load any cached
    /// scrobbles from the previous run.
    pub async fn set_retry_queue(inner: &DaemonInner) {
        let path = inner.config.data_dir.join("scrobble-queue.json");
        let mut lastfm = inner.lastfm.lock().await;
        lastfm.set_retry_path(path);
    }

    pub async fn auth_url(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let lastfm = inner.lastfm.lock().await;
        if let Some(url) = lastfm.auth_url() {
            Ok(DaemonRes::LastfmAuthUrlRes { url })
        } else {
            Ok(DaemonRes::Error {
                message: "Last.fm API key/secret not configured".into(),
            })
        }
    }

    /// Exchange a web-auth token for a (persisted) session key, clearing any
    /// recorded link error. Shared by the IPC `authenticate` call and the
    /// daemon-hosted OAuth flow task so both paths run the same exchange.
    pub async fn authenticate(inner: &DaemonInner, token: &str) -> Result<DaemonRes, CoreError> {
        match Self::exchange_session(inner, token).await {
            Ok(()) => Ok(DaemonRes::Ok),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    /// Start the Last.fm OAuth link flow entirely daemon-side: bind the
    /// loopback callback *before* returning the authorize URL (so the redirect
    /// never lands on a dead port), then capture the returning `token`,
    /// exchange it for a session key, and push [`DaemonEvent::LastfmStatusChanged`]
    /// so the TUI dismisses its prompt and proceeds to a playback-ready state
    /// as soon as the exchange completes — no client-side polling, no
    /// client-bound callback socket. Failures (timeout, exchange error) push
    /// the same event with the reason carried by the next status poll. The
    /// TUI's only hooks — start → url → status event — are identical to the
    /// Spotify flow, so future providers plug into the same contract.
    pub async fn oauth_start(inner: &Arc<DaemonInner>, port: u16) -> Result<DaemonRes, CoreError> {
        let url = {
            let lastfm = inner.lastfm.lock().await;
            lastfm.auth_url().ok_or_else(|| {
                CoreError::Daemon(
                    "Last.fm API key/secret not configured; enter them in the setup form first"
                        .into(),
                )
            })?
        };
        // Abort any previous pending flow so its listener socket is freed.
        if let Some(handle) = inner.oauth_lastfm_task.lock().await.take() {
            handle.abort();
        }
        // A fresh attempt starts clean: any error from an earlier (failed) flow
        // must not leak into the status the new flow's completion event polls.
        *inner.lastfm_error.lock().await = None;
        let listener = bind_callback(port, OAUTH_TIMEOUT)
            .await
            .map_err(|e| CoreError::Daemon(format!("Last.fm callback server: {e}")))?;
        let inner2 = Arc::clone(inner);
        let handle = tokio::spawn(async move {
            match listener.accept_param("token", None).await {
                Ok(token) => match Lastfm::exchange_session(&inner2, token.trim()).await {
                    Ok(()) => {
                        info!("last.fm oauth link complete (session key stored)");
                    }
                    Err(e) => {
                        warn!("last.fm oauth exchange failed: {e}");
                        *inner2.lastfm_error.lock().await = Some(e);
                    }
                },
                Err(e) => {
                    warn!("last.fm oauth link failed: {e}");
                    *inner2.lastfm_error.lock().await = Some(e);
                }
            }
            // One status event covers both outcomes: `ready` dismisses the
            // TUI prompt and proceeds; a `lastfm_error` reasons it inline.
            let _ = inner2.event_tx.send(DaemonEvent::LastfmStatusChanged);
        });
        *inner.oauth_lastfm_task.lock().await = Some(handle);
        Ok(DaemonRes::LastfmAuthUrlRes { url })
    }

    /// Exchange a token for a session key, persist it, and clear any recorded
    /// link error.
    pub(crate) async fn exchange_session(inner: &DaemonInner, token: &str) -> Result<(), String> {
        let mut lastfm = inner.lastfm.lock().await;
        let session_key =
            match tokio::time::timeout(Duration::from_secs(10), lastfm.authenticate(token)).await {
                Ok(Ok(key)) => key,
                Ok(Err(e)) => return Err(e),
                Err(_elapsed) => return Err("Last.fm authentication timed out".into()),
            };
        drop(lastfm);
        let mut state = inner.state.write().await;
        state.scrobble.session_token = Some(session_key);
        drop(state);
        Daemon::save_state(inner);
        *inner.lastfm_error.lock().await = None;
        Ok(())
    }

    pub async fn status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let lastfm = inner.lastfm.lock().await;
        let current_track = inner.state.read().await.current_track.clone();
        let state = inner.state.read().await;
        let enabled = state.scrobble.enabled;
        let api_key = state
            .scrobble
            .api_key
            .clone()
            .or_else(|| get_secret(LASTFM_API_KEY));
        let session_token = state.scrobble.session_token.clone();
        drop(state);
        let ready = lastfm.is_ready().await;
        // `loved` reports only for the currently playing track; a fresh track
        // (or none) always reports false.
        let loved = {
            let store = inner.lastfm_loved.lock().unwrap();
            current_track
                .as_ref()
                .map(track_love_key)
                .is_some_and(|cur| store.as_ref().is_some_and(|(k, loved)| *k == cur && *loved))
        };
        Ok(DaemonRes::LastfmStatusRes {
            enabled,
            api_key,
            session_token,
            ready,
            loved,
            error: inner.lastfm_error.lock().await.clone(),
        })
    }

    /// Love the current track. A loved track is scrobbled immediately — even
    /// below the normal play threshold — so a quick skip never loses it.
    pub async fn love(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        Self::set_loved(inner, true).await
    }

    /// Un-love the current track.
    pub async fn unlove(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        Self::set_loved(inner, false).await
    }

    pub(crate) async fn set_loved(inner: &DaemonInner, love: bool) -> Result<DaemonRes, CoreError> {
        let track = inner.state.read().await.current_track.clone();
        let Some(track) = track else {
            return Ok(DaemonRes::Error {
                message: "No track is currently playing".into(),
            });
        };
        {
            let mut store = inner.lastfm_loved.lock().unwrap();
            *store = Some((track_love_key(&track), love));
        }
        // Record the loved state even when the daemon isn't authenticated so
        // the status stays truthful; only the network call needs credentials.
        let result = {
            let lastfm = inner.lastfm.lock().await;
            if !lastfm.is_ready().await {
                Ok(Ok(()))
            } else if love {
                tokio::time::timeout(Duration::from_secs(10), lastfm.love(&track)).await
            } else {
                tokio::time::timeout(Duration::from_secs(10), lastfm.unlove(&track)).await
            }
        };
        if love {
            // Immediate scrobble: a loved track counts even pre-threshold.
            Cmd::scrobble_track(inner, &track, 0.0).await;
        }
        match result {
            Ok(Ok(())) => Ok(DaemonRes::Ok),
            Ok(Err(e)) => Ok(DaemonRes::Error { message: e }),
            Err(_) => Ok(DaemonRes::Error {
                message: "Last.fm request timed out".into(),
            }),
        }
    }

    pub async fn clear(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut lastfm = inner.lastfm.lock().await;
        lastfm.clear_session().await;
        delete_secret(LASTFM_API_KEY);
        delete_secret(LASTFM_API_SECRET);
        let mut state = inner.state.write().await;
        state.scrobble.enabled = false;
        state.scrobble.api_key = None;
        state.scrobble.session_token = None;
        drop(state);
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }
}
