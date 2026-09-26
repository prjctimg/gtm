use std::time::{Duration, Instant};

use gtm::shared::CoreError;
use gtm::shared::ipc::DaemonRes;
use gtm::shared::radio::{RadioTrack, RadioTracklist};
use tracing::warn;

use super::{DaemonInner, resolve_remote};
use crate::tracklist::{client as track_client, fetch as track_fetch};

/// How often a playing station's tracklist is refetched. Station pages refresh
/// on the order of tens of seconds, and a tracklist is only a display
/// refinement, so this stays well below the rate that would add real traffic.
const REFRESH: Duration = Duration::from_secs(30);

/// What the daemon remembers about the playing station between refreshes: which
/// station it is bound to and when the next fetch is due. Deliberately holds no
/// list, so a refresh can run without a lock across network I/O and the tick
/// never waits on a slow source.
pub(crate) struct Bound {
    station: String,
    next: Instant,
}

impl Bound {
    pub fn new(station: &str) -> Self {
        Self {
            station: station.to_string(),
            next: Instant::now(),
        }
    }

    pub fn station(&self) -> &str {
        &self.station
    }

    /// Whether `station` is a different station, so the binding must change.
    pub fn switched(&self, station: &str) -> bool {
        self.station != station
    }

    /// Whether the refresh interval has elapsed. Marks the next deadline as a
    /// side effect, so a failing source retries on the same cadence as a
    /// working one instead of on every tick.
    pub fn due(&mut self) -> bool {
        if Instant::now() < self.next {
            return false;
        }
        self.next = Instant::now() + REFRESH;
        true
    }
}

/// Resolve `station_id` to a playable stream URL and fetch its tracklist.
/// Stream resolution goes through the same path as playback, so the source is
/// selected from the host actually being decoded.
pub(crate) async fn load(inner: &DaemonInner, station_id: &str) -> RadioTracklist {
    let resolved = match resolve_remote(inner, &format!("radio://{station_id}")).await {
        Ok(r) => r,
        Err(e) => {
            warn!("radio tracklist resolve: {e}");
            return RadioTracklist::default();
        }
    };
    // A `radios.toml` tracklist URL bypasses the registry entirely, which is
    // how a station on an unrecognised control panel still gets a queue.
    let override_url = gtm::shared::custom::parse_custom_id(station_id)
        .and_then(|i| gtm::shared::custom::station_by_index(i).ok().flatten())
        .and_then(|s| s.tracklist);
    match track_fetch(
        &track_client(),
        &resolved.url,
        station_id,
        override_url.as_deref(),
    )
    .await
    {
        Ok(list) => list,
        Err(e) => {
            warn!("radio tracklist: {e}");
            RadioTracklist::default()
        }
    }
}

/// Fetch a station's tracklist on demand, for the client opening the queue
/// view. Independent of the playing station's cached binding, so browsing one
/// station never disturbs what is on air.
pub(crate) async fn fetch(inner: &DaemonInner, station_id: &str) -> Result<DaemonRes, CoreError> {
    Ok(DaemonRes::RadioTracklistRes {
        list: Box::new(load(inner, station_id).await),
    })
}

/// The title on air and the artist half of it, from a list plus the stream's
/// ICY `StreamTitle`.
///
/// ICY is authoritative for what is playing, so the artist is read from the
/// list row that title matches rather than from the list's own leading entry,
/// which can lag by up to one refresh. A station publishing no ICY metadata
/// falls back to the leading entry, the same convention the station's own page
/// uses.
pub(crate) fn now_split(list: &RadioTracklist, icy: Option<&str>) -> (String, Option<String>) {
    let artist =
        |t: &RadioTrack| Some(t.artist.clone()).filter(|a| !a.is_empty());
    match icy {
        Some(title) if !title.is_empty() => {
            let at = list.match_title(title);
            match list.tracks.get(at) {
                Some(t) => (title.to_string(), artist(t)),
                None => (title.to_string(), None),
            }
        }
        _ => match list.now() {
            Some(t) => (t.title.clone(), artist(t)),
            None => (String::new(), None),
        },
    }
}
