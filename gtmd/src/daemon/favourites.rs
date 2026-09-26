use super::*;

pub(crate) struct Favourites;

impl Favourites {
    pub async fn list(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.get_favourites()
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

    pub async fn add(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    pub async fn remove(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }
}
