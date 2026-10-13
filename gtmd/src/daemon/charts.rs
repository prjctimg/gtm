use super::*;

use futures::StreamExt;
use gtm::providers::charts::ChartTrack;

/// Chart providers (Spotify first; further sources plug in via the same trait).
pub(crate) struct Charts;

impl Charts {
    pub async fn sources(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let registry = inner.charts.lock().await;
        Ok(DaemonRes::ChartsSourcesRes {
            sources: registry.sources(),
        })
    }

    pub async fn list(
        inner: &DaemonInner,
        source_id: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        let registry = inner.charts.lock().await;
        match source_id {
            Some(id) => {
                if let Some(provider) = registry.get(&id) {
                    match provider.list_charts().await {
                        Ok(charts) => Ok(DaemonRes::ChartsListRes { charts }),
                        Err(e) => Ok(DaemonRes::Error {
                            message: format!("charts list failed: {e}"),
                        }),
                    }
                } else {
                    Ok(DaemonRes::Error {
                        message: format!("unknown chart source: {id}"),
                    })
                }
            }
            None => match registry.list_all_charts().await {
                Ok(charts) => Ok(DaemonRes::ChartsListRes { charts }),
                Err(e) => Ok(DaemonRes::Error {
                    message: format!("charts list failed: {e}"),
                }),
            },
        }
    }

    pub async fn tracks(
        inner: &DaemonInner,
        source_id: String,
        chart_id: String,
    ) -> Result<DaemonRes, CoreError> {
        let fetched = {
            let registry = inner.charts.lock().await;
            registry.chart_tracks(&source_id, &chart_id).await
        };
        match fetched {
            Ok(tracks) => Ok(DaemonRes::ChartsTracksRes {
                tracks: fill_covers(tracks).await,
            }),
            Err(e) => Ok(DaemonRes::Error {
                message: format!("charts tracks failed: {e}"),
            }),
        }
    }
}

/// Fill in chart art the provider itself could not supply, through Deezer.
///
/// The iTunes feed carries no artwork of its own — a row's cover exists only if
/// its track id came back from the batched lookup — so every unmatched id left
/// a row that rendered as a blank card. Deezer is the one catalog that can be
/// asked by artist and title without a key, which is exactly the information a
/// chart row already carries. The Deezer chart provider needs none of this: it
/// reads artwork straight off its own payload, so its rows are already whole and
/// the backfill returns immediately.
///
/// Bounded rather than sequential: a chart is a hundred rows, and the point of
/// resolving at all is that every one of them shows art, not the first few. The
/// searcher rate-limits itself per call, so the cap is what stops a chart open
/// from turning into a hundred simultaneous requests.
async fn fill_covers(tracks: Vec<ChartTrack>) -> Vec<ChartTrack> {
    const CAP: usize = 8;
    let missing: Vec<(usize, String, String, f64)> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.cover_url.is_none() && !t.artists.is_empty() && !t.title.is_empty())
        .map(|(i, t)| {
            (
                i,
                t.artists.clone(),
                t.title.clone(),
                t.duration_ms.unwrap_or(0) as f64 / 1000.0,
            )
        })
        .collect();
    if missing.is_empty() {
        return tracks;
    }
    let found: Vec<(usize, String)> = futures::stream::iter(missing)
        .map(|(i, artist, title, secs)| async move {
            let hit = DeezerSearch::new()
                .search(&artist, &title, secs)
                .await
                .ok()
                .flatten();
            hit.and_then(|h| h.cover_url.map(|u| (i, u)))
        })
        .buffer_unordered(CAP)
        .filter_map(|f| async move { f })
        .collect()
        .await;
    let mut tracks = tracks;
    for (i, url) in found {
        tracks[i].cover_url = Some(url);
    }
    tracks
}
