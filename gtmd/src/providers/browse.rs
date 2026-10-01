// Music browse: free-text search, artist pages, album tracklists.
//
// Spotify cannot supply this. Its Web API removed artist-contents endpoints in
// 2026 for developer-mode integrations, so there is no endpoint to ask for an
// artist's albums or an album's tracklist — the log line `Spotify no longer
// exposes artist contents through the public API` is the client reporting
// exactly that. Deezer's public API is keyless and answers all three questions,
// and it is already a chart provider here, so this adds no new account.
//
// Every result is metadata, not audio: a row names a track and a provider that
// can be resolved into one, and playback goes through the same resolve path the
// chart rows already use.

use serde::Deserialize;

const API: &str = "https://api.deezer.com";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

use gtm::shared::chart::{
    AlbumPage, ArtistPage, BrowseAlbum, BrowseArtist, BrowseHit, BrowseTrack,
};

/// Why a browse request could not be answered.
///
/// A plain `String` error would read the same in the UI, but these are matched
/// on: an empty query and an unmatched one are the user's typing and the
/// catalogue's contents, and only the second is worth telling apart.
#[derive(Debug, PartialEq, Eq)]
pub enum BrowseError {
    Empty,
    NoMatch(String),
    Network(String),
    Parse(String),
}

impl std::fmt::Display for BrowseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowseError::Empty => write!(f, "type something to search for"),
            BrowseError::NoMatch(t) => write!(f, "nothing matched {t:?}"),
            BrowseError::Network(m) => write!(f, "network: {m}"),
            BrowseError::Parse(m) => write!(f, "bad response: {m}"),
        }
    }
}

pub struct Browse;

async fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, BrowseError> {
    let resp = reqwest::Client::new()
        .get(format!("{API}{path}"))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| BrowseError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(BrowseError::Network(format!("HTTP {}", resp.status())));
    }
    resp.json()
        .await
        .map_err(|e| BrowseError::Parse(e.to_string()))
}

impl Browse {
    /// Search for tracks, albums and artists in one pass.
    ///
    /// Three requests rather than one: the API has no combined endpoint, and
    /// interleaving the three result sets would have to invent a ranking the
    /// source does not give. Grouped instead, in that order, so the list reads
    /// as "the songs, then the releases, then the people".
    pub async fn search(term: &str) -> Result<Vec<BrowseHit>, BrowseError> {
        let term = term.trim();
        if term.is_empty() {
            return Err(BrowseError::Empty);
        }
        let q = urlencoding::encode(term);

        let tracks: DzPage<DzTrack> = get(&format!("/search?q={q}&limit=25")).await?;
        let albums: DzPage<DzAlbum> = get(&format!("/search/album?q={q}&limit=15")).await?;

        // `/search/artist` returns people directly — id, name, picture — not
        // tracks with a nested artist, so it deserialises as `DzArtist`.
        // Failing soft is deliberate: an artist is the least useful of the three
        // results, and losing it should not lose the tracks.
        let artists: DzPage<DzArtist> = get(&format!("/search/artist?q={}&limit=15", q))
            .await
            .unwrap_or(DzPage { data: None });

        let mut out = Vec::new();
        for t in tracks.into_vec() {
            out.push(BrowseHit::Track(t.into()));
        }
        for a in albums.into_vec() {
            out.push(BrowseHit::Album(a.into()));
        }
        let mut seen = std::collections::HashSet::new();
        for a in artists.into_vec() {
            if a.id == 0 || !seen.insert(a.id) {
                continue;
            }
            out.push(BrowseHit::Artist(BrowseArtist {
                id: a.id,
                name: a.name,
                image_url: a.picture_xl,
                album_count: a.nb_album,
            }));
        }

        if out.is_empty() {
            Err(BrowseError::NoMatch(term.to_string()))
        } else {
            Ok(out)
        }
    }

    /// An artist's top tracks and releases.
    pub async fn artist(id: u64) -> Result<ArtistPage, BrowseError> {
        let a: DzArtist = get(&format!("/artist/{id}")).await?;
        // `top` is the only per-artist track list this API exposes, and it is
        // ranked rather than complete. The client labels it a top list rather
        // than presenting it as a discography.
        let top: DzPage<DzTrack> = get(&format!("/artist/{id}/top?limit=25")).await?;
        let rel: DzPage<DzAlbum> = get(&format!("/artist/{id}/albums?limit=50")).await?;
        Ok(ArtistPage {
            artist: BrowseArtist {
                id,
                name: a.name,
                image_url: a.picture_xl,
                album_count: a.nb_album,
            },
            top_tracks: top.into_vec().into_iter().map(Into::into).collect(),
            albums: rel.into_vec().into_iter().map(Into::into).collect(),
        })
    }

    /// A full album tracklist — the one question the Spotify API can no longer
    /// be asked.
    pub async fn album(id: u64) -> Result<AlbumPage, BrowseError> {
        let a: DzAlbum = get(&format!("/album/{id}")).await?;
        let tracks: DzPage<DzTrack> = get(&format!("/album/{id}/tracks?limit=100")).await?;
        Ok(AlbumPage {
            album: a.into(),
            tracks: tracks.into_vec().into_iter().map(Into::into).collect(),
        })
    }
}

// ─── wire shapes ───

#[derive(Deserialize)]
struct DzPage<T> {
    /// A missing key is not an error here: Deezer omits `data` on some shapes
    /// rather than returning it empty, and an empty page is the right reading.
    /// `#[serde(default)]` would need `T: Default`, which the row types are not.
    data: Option<Vec<T>>,
}

impl<T> DzPage<T> {
    fn into_vec(self) -> Vec<T> {
        self.data.unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct DzArtist {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    picture_xl: Option<String>,
    #[serde(rename = "nb_album", default)]
    nb_album: Option<u64>,
}

#[derive(Deserialize)]
struct DzAlbum {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cover_xl: Option<String>,
    #[serde(rename = "nb_tracks", default)]
    nb_tracks: Option<u64>,
    #[serde(default)]
    artist: Option<DzArtist>,
}

impl From<DzAlbum> for BrowseAlbum {
    fn from(a: DzAlbum) -> Self {
        BrowseAlbum {
            id: a.id,
            title: a.title,
            artist: a.artist.map(|x| x.name).unwrap_or_default(),
            image_url: a.cover_xl,
            track_count: a.nb_tracks,
        }
    }
}

#[derive(Deserialize)]
struct DzTrack {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    artist: Option<DzArtist>,
    #[serde(default)]
    album: Option<DzAlbum>,
}

impl From<DzTrack> for BrowseTrack {
    fn from(t: DzTrack) -> Self {
        let cover = t
            .album
            .as_ref()
            .and_then(|a| a.cover_xl.clone())
            .or_else(|| t.artist.as_ref().and_then(|a| a.picture_xl.clone()));
        BrowseTrack {
            id: t.id,
            title: t.title,
            artist: t.artist.map(|a| a.name).unwrap_or_default(),
            album: t.album.map(|a| a.title),
            duration_secs: t.duration,
            preview_url: t.preview,
            image_url: cover,
        }
    }
}
