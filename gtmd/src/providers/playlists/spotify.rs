// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist writes against the Spotify Web API.
//
// This is free software released under the GPL-3.0 license.

use std::sync::Arc;

use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{
    FullPlaylist, ItemPositions, LibraryId, PlayableId, PlayableItem,
    PlaylistId as SpotifyPlaylistId, TrackId, UserId,
};
use tracing::warn;

use gtm::providers::playlist::{
    NewPlaylist, PlaylistCaps, PlaylistError, PlaylistId, PlaylistItem, PlaylistProvider,
    PlaylistSlot, SPOTIFY_SOURCE,
};

use crate::daemon::DaemonInner;

/// A cloned Web API client, so no manager lock is held across a call.
type Client = rspotify::AuthCodePkceSpotify;

/// Spotify's playlists, as a `PlaylistProvider`.
///
/// The reads are not here. Spotify embeds its tracks in the playlist cache the
/// sync builds, with a position per track that `SpotifyResolve` indexes by;
/// duplicating that into a provider-neutral list means shipping a second copy of
/// every playlist and renumbering positions nothing else agrees with. So reading
/// stays where it is and this trait is the verbs that really are the same
/// everywhere.
pub(crate) struct SpotifyPlaylists {
    inner: Arc<DaemonInner>,
}

impl SpotifyPlaylists {
    pub(crate) fn new(inner: Arc<DaemonInner>) -> Self {
        Self { inner }
    }

    /// A cloned Web API client, so no manager lock is held across the network.
    ///
    /// The lock discipline is not incidental: it is released here, before every
    /// await, because holding it across a call is what wedged playback, cover art
    /// and search together once already.
    async fn client(&self) -> Result<Client, PlaylistError> {
        self.inner
            .spotify
            .lock()
            .await
            .client()
            .ok_or_else(|| PlaylistError::NotPermitted("spotify not linked".into()))
    }

    /// The provider's own id, checked against the source so a playlist id from
    /// another provider is refused rather than sent to Spotify.
    fn split<'a>(id: &'a PlaylistId) -> Result<SpotifyPlaylistId<'a>, PlaylistError> {
        if id.source != SPOTIFY_SOURCE {
            return Err(PlaylistError::NotPermitted(format!(
                "{} is not a spotify playlist",
                id.source
            )));
        }
        SpotifyPlaylistId::from_id(id.id.as_str())
            .map_err(|e| PlaylistError::Rejected(format!("bad playlist id: {e}")))
    }

    async fn user_id(client: &Client) -> Result<UserId<'static>, PlaylistError> {
        client
            .current_user()
            .await
            .map(|u| u.id.to_owned())
            .map_err(classify)
    }

    /// Re-read the playlist list after a write.
    ///
    /// Without this the pane shows the list as it was. `playlist_add` had this
    /// problem on its own and six more verbs would have multiplied it -- a write
    /// that succeeds and a list that does not change reads as a write that did
    /// nothing. Best-effort: a refresh that fails leaves the previous list
    /// rather than an empty one.
    async fn after_write(&self) {
        if let Err(e) = self.inner.spotify.lock().await.sync().await {
            warn!("spotify: the write succeeded but the playlist list did not refresh: {e}");
        }
    }
}

/// What a Spotify error means for a playlist verb.
///
/// The token endpoint's own failures are the ones worth naming: a dead grant is
/// answered by re-opening the authorize page, and an opaque string reaching the
/// UI instead is what let a revoked account sit there reading "Connected".
fn classify(err: impl std::fmt::Display) -> PlaylistError {
    let msg = err.to_string();
    if crate::daemon::is_dead_token(&msg) {
        PlaylistError::NotPermitted(format!("the stored grant was rejected: {msg}"))
    } else if msg.contains("status code 401") {
        PlaylistError::NotPermitted(format!("spotify refused the grant: {msg}"))
    } else if msg.contains("status code 429") {
        PlaylistError::Unreachable("spotify's quota for this app is spent".into())
    } else {
        PlaylistError::Rejected(msg)
    }
}

/// A playlist item as the add and remove endpoints want it.
///
/// `PlayableId` has no parser of its own, so the uri goes through `TrackId`,
/// which does -- and an episode or a show uri is refused with the same message a
/// malformed one gets, which is the honest answer for a playlist this app fills
/// with songs.
fn playable<'a>(item: &'a PlaylistItem) -> Result<PlayableId<'a>, PlaylistError> {
    TrackId::from_uri(item.item.as_str())
        .map(PlayableId::Track)
        .map_err(|e| PlaylistError::Rejected(format!("bad track uri {}: {e}", item.item)))
}

fn full_to_id(full: FullPlaylist) -> Result<PlaylistId, PlaylistError> {
    Ok(PlaylistId::new(SPOTIFY_SOURCE, full.id.to_string()))
}

#[async_trait::async_trait]
impl PlaylistProvider for SpotifyPlaylists {
    fn source_id(&self) -> &str {
        SPOTIFY_SOURCE
    }

    fn display_name(&self) -> &str {
        "Spotify"
    }

    fn caps(&self) -> PlaylistCaps {
        // Every verb is implemented and every scope each one needs is already
        // requested at link time, so nothing here is aspirational. That is the
        // difference from the charts provider, which reported itself configured
        // while its endpoints had been removed: these calls exist today.
        PlaylistCaps::ALL
    }

    async fn create(&self, spec: NewPlaylist) -> Result<PlaylistId, PlaylistError> {
        let client = self.client().await?;
        let name = spec
            .name
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| PlaylistError::Rejected("a playlist needs a name".into()))?;
        let user = Self::user_id(&client).await?;
        let full = client
            .user_playlist_create(
                user,
                &name,
                spec.public,
                // Collaborative is deliberately never offered: it needs the
                // playlist to be private, and nothing in the spec asked for that.
                Some(false),
                spec.description.as_deref(),
            )
            .await
            .map_err(classify)?;
        let id = full_to_id(full)?;
        self.after_write().await;
        Ok(id)
    }

    async fn rename(&self, id: &PlaylistId, spec: NewPlaylist) -> Result<(), PlaylistError> {
        let client = self.client().await?;
        let target = Self::split(id)?;
        client
            .playlist_change_detail(
                target,
                spec.name.as_deref(),
                spec.public,
                spec.description.as_deref(),
                None,
            )
            .await
            .map_err(classify)?;
        self.after_write().await;
        Ok(())
    }

    async fn add_items(
        &self,
        id: &PlaylistId,
        items: &[PlaylistItem],
        at: Option<u32>,
    ) -> Result<(), PlaylistError> {
        if items.is_empty() {
            return Err(PlaylistError::Rejected("nothing to add".into()));
        }
        let client = self.client().await?;
        let target = Self::split(id)?;
        let tracks = items.iter().map(playable).collect::<Result<Vec<_>, _>>()?;
        client
            .playlist_add_items(target, tracks, at)
            .await
            .map_err(classify)?;
        self.after_write().await;
        Ok(())
    }

    async fn remove_occurrences(
        &self,
        id: &PlaylistId,
        slots: &[PlaylistSlot],
    ) -> Result<(), PlaylistError> {
        if slots.is_empty() {
            return Err(PlaylistError::Rejected("nothing to remove".into()));
        }
        let client = self.client().await?;
        let target = Self::split(id)?;
        // `ItemPositions` borrows its positions, so they are gathered first and
        // the slices outlive the call.
        let positions: Vec<Vec<u32>> = slots.iter().map(|s| s.positions.clone()).collect();
        let spots = slots
            .iter()
            .zip(&positions)
            .map(
                |(slot, positions)| -> Result<ItemPositions<'_>, PlaylistError> {
                    Ok(ItemPositions {
                        id: playable(&slot.item)?,
                        positions: positions.as_slice(),
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?;
        client
            .playlist_remove_specific_occurrences_of_items(target, spots, None)
            .await
            .map_err(classify)?;
        self.after_write().await;
        Ok(())
    }

    async fn delete(&self, id: &PlaylistId) -> Result<(), PlaylistError> {
        let client = self.client().await?;
        let target = Self::split(id)?;
        // The Library API, not `playlist_unfollow`: Spotify deprecated the
        // latter in favour of this, and a deprecated call is one that stops
        // working without a compile error.
        client
            .library_remove([LibraryId::Playlist(target.clone())])
            .await
            .map_err(classify)?;
        self.after_write().await;
        Ok(())
    }

    async fn subscribe(&self, id: &PlaylistId, on: bool) -> Result<(), PlaylistError> {
        let client = self.client().await?;
        let target = Self::split(id)?;
        let lib = [LibraryId::Playlist(target.clone())];
        let sent = if on {
            client.library_add(lib).await
        } else {
            client.library_remove(lib).await
        };
        sent.map_err(classify)?;
        self.after_write().await;
        Ok(())
    }

    async fn copy(&self, id: &PlaylistId, name: Option<&str>) -> Result<PlaylistId, PlaylistError> {
        let client = self.client().await?;
        let target = Self::split(id)?;
        // Read, create, refill: the API has no server-side copy. A failure
        // halfway leaves a playlist named as asked but shorter than the
        // original, which is reported rather than passed off as a full copy.
        let source = client
            .playlist(target, None, None)
            .await
            .map_err(classify)?;
        let title = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} (copy)", source.name));
        let user = Self::user_id(&client).await?;
        let made = client
            .user_playlist_create(user, &title, Some(false), Some(false), None)
            .await
            .map_err(classify)?;

        // `items`, not the deprecated `tracks`: Spotify renamed the field in
        // its February 2026 migration and rspotify keeps the old one only for
        // deserialisation. `item`, likewise, replaces `track`.
        let uris: Vec<String> = source
            .items
            .items
            .iter()
            .filter_map(|it| it.item.as_ref())
            .filter_map(|t| match t {
                PlayableItem::Track(tr) => tr.id.as_ref().map(|t| t.to_string()),
                PlayableItem::Episode(_) | PlayableItem::Unknown(_) => None,
            })
            .collect();
        if !uris.is_empty() {
            let refs = uris
                .iter()
                .map(|u| TrackId::from_uri(u).map(|t| PlayableId::Track(t.to_owned())))
                .collect::<Result<Vec<_>, _>>()
                .map_err(classify)?;
            let made_raw = made.id.to_string();
            let made_id = SpotifyPlaylistId::from_id(made_raw.as_str())
                .map_err(|e| PlaylistError::Rejected(format!("minted an unusable id: {e}")))?;
            client
                .playlist_add_items(made_id, refs, None)
                .await
                .map_err(classify)?;
        }
        let out = PlaylistId::new(SPOTIFY_SOURCE, made.id.to_string());
        self.after_write().await;
        Ok(out)
    }
}
