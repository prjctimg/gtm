use super::*;

pub(crate) struct Search;

impl Search {
    pub async fn handle(inner: &DaemonInner, query: &str) -> Result<DaemonRes, CoreError> {
        let query = query.to_string();
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.search_tracks(&query)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(tracks) => Ok(DaemonRes::Tracks {
                tracks: Box::new(tracks),
            }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }
}
