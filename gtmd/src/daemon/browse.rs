use super::*;

use crate::providers::browse::{Browse, BrowseError};

/// Music browse: search, artist pages, album tracklists.
///
/// Stateless — every request is a fresh call to a keyless public API, so there
/// is no cache to keep coherent and nothing to invalidate. The chart registry
/// holds a lock for its providers because they carry session state; this has
/// none, which is why these handlers do not touch `inner` at all.
pub(crate) struct Browse_;

impl Browse_ {
    pub async fn search(term: &str) -> Result<DaemonRes, CoreError> {
        match Browse::search(term).await {
            Ok(hits) => Ok(DaemonRes::BrowseSearchRes { hits }),
            // An unmatched query is not a failure of the daemon; it is the
            // catalogue's answer, and the client renders it as an empty list
            // rather than an error. Only a real fault becomes one.
            Err(BrowseError::Empty) | Err(BrowseError::NoMatch(_)) => {
                Ok(DaemonRes::BrowseSearchRes { hits: Vec::new() })
            }
            Err(e) => Ok(DaemonRes::Error {
                message: format!("browse search failed: {e}"),
            }),
        }
    }

    pub async fn artist(artist_id: u64) -> Result<DaemonRes, CoreError> {
        match Browse::artist(artist_id).await {
            Ok(page) => Ok(DaemonRes::BrowseArtistRes {
                page: Box::new(page),
            }),
            Err(e) => Ok(DaemonRes::Error {
                message: format!("browse artist failed: {e}"),
            }),
        }
    }

    pub async fn album(album_id: u64) -> Result<DaemonRes, CoreError> {
        match Browse::album(album_id).await {
            Ok(page) => Ok(DaemonRes::BrowseAlbumRes {
                page: Box::new(page),
            }),
            Err(e) => Ok(DaemonRes::Error {
                message: format!("browse album failed: {e}"),
            }),
        }
    }
}
