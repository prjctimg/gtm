// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Cover art fetching from Deezer API with disk and LRU cache
//
// This is free software released under the GPL-3.0 license.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use lru::LruCache;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use tokio::sync::Mutex;
use tracing::warn;

const CACHE_SIZE: usize = 500;
const ARTIST_CACHE_SIZE: usize = 200;
const DEEZER_API: &str = "https://api.deezer.com/search";
const RATE_LIMIT_MS: u64 = 200;
const MIN_COVER_DIM: u32 = 300;

/// Which artwork source(s) to consult, in the requested order. `Auto` is the
/// default: MusicBrainz/Cover Art Archive first, with Spotify preferred over
/// everything whenever an account is linked (higher resolution artwork).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverProvider {
    #[default]
    Auto,
    Deezer,
    Musicbrainz,
    Spotify,
}

impl CoverProvider {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "deezer" => Self::Deezer,
            "musicbrainz" | "mb" | "musicbrainz/cover-art-archive" => Self::Musicbrainz,
            "spotify" => Self::Spotify,
            _ => Self::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverData {
    pub data: Vec<u8>,
    pub mime: String,
}

pub struct CoverCache {
    memory: Arc<Mutex<LruCache<String, CoverData>>>,
    artist_memory: Arc<Mutex<LruCache<String, CoverData>>>,
    cache_dir: PathBuf,
    client: Client,
}

impl CoverCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(cache_dir.join("covers")).ok();
        fs::create_dir_all(cache_dir.join("artist_covers")).ok();
        Self {
            memory: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_SIZE).unwrap(),
            ))),
            artist_memory: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(ARTIST_CACHE_SIZE).unwrap(),
            ))),
            cache_dir,
            client: Client::new(),
        }
    }

    pub fn cover_too_small(data: &[u8]) -> bool {
        match image::load_from_memory(data) {
            Ok(img) => img.width() < MIN_COVER_DIM || img.height() < MIN_COVER_DIM,
            Err(_) => true,
        }
    }

    /// Normalize arbitrary album-cover bytes to a single standard size for the
    /// whole app: centre-cropped (never distorted) 500x500 JPEG at quality 90.
    /// Returns `None` when the input can't be decoded so callers can keep the
    /// original bytes.
    pub fn normalize_cover(data: &[u8]) -> Option<Vec<u8>> {
        let img = image::load_from_memory(data).ok()?;
        if img.width() == 0 || img.height() == 0 {
            return None;
        }
        let side = img.width().min(img.height());
        let x = (img.width() - side) / 2;
        let y = (img.height() - side) / 2;
        let img = img.crop_imm(x, y, side, side);
        let img = if img.width() == 500 && img.height() == 500 {
            img
        } else {
            img.resize(500, 500, image::imageops::FilterType::CatmullRom)
        };
        let mut out = std::io::Cursor::new(Vec::new());
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        img.write_with_encoder(enc).ok()?;
        Some(out.into_inner())
    }

    pub fn cache_key(artist: &str, album: &str) -> String {
        let mut h = Sha256::new();
        h.update(format!("{}:{}", artist, album).as_bytes());
        hex::encode(&h.finalize()[..8])
    }

    /// Insert externally-provided cover bytes (e.g. from Spotify) into both
    /// caches so a later fetch is served from disk/memory instantly.
    pub async fn put_cover(&self, artist: &str, album: &str, bytes: Vec<u8>) {
        if bytes.is_empty() || Self::cover_too_small(&bytes) {
            return;
        }
        let bytes = Self::normalize_cover(&bytes).unwrap_or(bytes);
        let artist = if artist.is_empty() {
            "Unknown Artist"
        } else {
            artist
        };
        let album = if album.is_empty() {
            "Unknown Album"
        } else {
            album
        };
        let key = Self::cache_key(artist, album);
        let cd = CoverData {
            mime: "image/jpeg".to_string(),
            data: bytes.clone(),
        };
        let disk = self.disk_path(&key);
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        if let Err(e) = fs::write(&disk, &bytes) {
            warn!("Failed to write cover to disk {disk:?}: {e}");
        }
        let mut mem = self.memory.lock().await;
        mem.put(key, cd);
    }

    /// Insert externally-provided artist image bytes into both artist caches.
    pub async fn put_artist_image(&self, artist: &str, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let artist = artist.trim();
        if artist.is_empty() {
            return;
        }
        let key = Self::artist_key(artist);
        let cd = CoverData {
            mime: "image/jpeg".to_string(),
            data: bytes.clone(),
        };
        let disk = self.artist_disk_path(&key);
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        if let Err(e) = fs::write(&disk, &bytes) {
            warn!("Failed to write artist image to disk {disk:?}: {e}");
        }
        let mut mem = self.artist_memory.lock().await;
        mem.put(key, cd);
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join("covers").join(format!("{key}.jpg"))
    }

    pub async fn get_cover(
        &mut self,
        artist: &str,
        album: &str,
        provider: CoverProvider,
    ) -> Option<CoverData> {
        let artist = if artist.is_empty() {
            "Unknown Artist"
        } else {
            artist
        };
        let album = if album.is_empty() {
            "Unknown Album"
        } else {
            album
        };
        let key = Self::cache_key(artist, album);

        {
            let mut mem = self.memory.lock().await;
            if let Some(c) = mem.get(&key)
                && !Self::cover_too_small(&c.data)
            {
                return Some(c.clone());
            }
        }

        let disk = self.disk_path(&key);
        if disk.exists()
            && let Ok(data) = fs::read(&disk)
        {
            if !Self::cover_too_small(&data) {
                let cd = CoverData {
                    mime: "image/jpeg".to_string(),
                    data,
                };
                let mut mem = self.memory.lock().await;
                mem.put(key, cd.clone());
                return Some(cd);
            }
            // Small cover on disk: remove it and re-fetch from Deezer
            fs::remove_file(&disk).ok();
        }

        // An explicit Deezer choice leads with Deezer, everything else (the
        // default) leads with MusicBrainz so original artwork wins over
        // Deezer's often re-scaled/re-sampled copies.
        let (first, second) = match provider {
            CoverProvider::Deezer => (CoverProvider::Deezer, CoverProvider::Musicbrainz),
            _ => (CoverProvider::Musicbrainz, CoverProvider::Deezer),
        };
        let mut cd = None;
        for p in [first, second] {
            if cd.is_some() {
                break;
            }
            cd = match p {
                CoverProvider::Deezer => self.fetch_from_deezer(artist, album, &key).await,
                CoverProvider::Musicbrainz => {
                    self.fetch_from_musicbrainz(artist, album, &key).await
                }
                CoverProvider::Auto | CoverProvider::Spotify => continue,
            };
        }
        if let Some(ref cd) = cd {
            let mut mem = self.memory.lock().await;
            mem.put(key, cd.clone());
        }
        cd
    }

    /// MusicBrainz / Cover Art Archive lookup (original artwork).
    async fn fetch_from_musicbrainz(
        &self,
        artist: &str,
        album: &str,
        key: &str,
    ) -> Option<CoverData> {
        let mb = crate::musicbrainz::MusicBrainz::new();
        let found = mb.find_album(artist, album).await.ok().flatten()?;
        let bytes = mb.download_cover(&found.release_group_id).await?;
        let bytes = Self::normalize_cover(&bytes).unwrap_or(bytes);
        let disk = self.disk_path(key);
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        if let Err(e) = fs::write(&disk, &bytes) {
            warn!("Failed to write MB cover to disk {disk:?}: {e}");
        }
        Some(CoverData {
            data: bytes,
            mime: "image/jpeg".to_string(),
        })
    }

    pub async fn get_artist_image(&mut self, artist: &str) -> Option<CoverData> {
        let artist = if artist.is_empty() {
            return None;
        } else {
            artist
        };
        let key = Self::artist_key(artist);
        {
            let mut mem = self.artist_memory.lock().await;
            if let Some(c) = mem.get(&key) {
                return Some(c.clone());
            }
        }
        let disk = self.artist_disk_path(&key);
        if disk.exists()
            && let Ok(data) = fs::read(&disk)
        {
            let cd = CoverData {
                mime: "image/jpeg".to_string(),
                data,
            };
            let mut mem = self.artist_memory.lock().await;
            mem.put(key, cd.clone());
            return Some(cd);
        }
        let deezer = crate::deezer::DeezerSearch::new();
        let img_bytes = deezer.artist_image(artist).await?;
        let cd = CoverData {
            data: img_bytes.clone(),
            mime: "image/jpeg".to_string(),
        };
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        if let Err(e) = fs::write(&disk, &img_bytes) {
            warn!("Failed to write artist image to disk {disk:?}: {e}");
        }
        let mut mem = self.artist_memory.lock().await;
        mem.put(key, cd.clone());
        Some(cd)
    }

    fn artist_key(artist: &str) -> String {
        let mut h = Sha256::new();
        h.update(format!("artist:{artist}").as_bytes());
        hex::encode(&h.finalize()[..8])
    }

    fn artist_disk_path(&self, key: &str) -> PathBuf {
        self.cache_dir
            .join("artist_covers")
            .join(format!("{key}.jpg"))
    }

    async fn fetch_from_deezer(&self, artist: &str, album: &str, key: &str) -> Option<CoverData> {
        let query = format!(
            "artist:\"{}\" album:\"{}\"",
            urlencoding(artist),
            urlencoding(album)
        );

        tokio::time::sleep(std::time::Duration::from_millis(RATE_LIMIT_MS)).await;

        let resp = match self
            .client
            .get(DEEZER_API)
            .query(&[("q", &query)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Deezer API request failed for {artist}/{album}: {e}");
                return None;
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!("Deezer JSON parse failed for {artist}/{album}: {e}");
                return None;
            }
        };
        let data = match json.get("data").and_then(|d| d.as_array()) {
            Some(d) => d,
            None => {
                warn!("Deezer returned no data for {artist}/{album}");
                return None;
            }
        };
        let first = match data.first() {
            Some(f) => f,
            None => {
                warn!("Deezer empty results for {artist}/{album}");
                return None;
            }
        };

        let cover_url = first
            .get("cover_big")
            .or_else(|| first.get("cover_medium"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if cover_url.is_empty() {
            warn!("Deezer cover URL empty for {artist}/{album}");
            return None;
        }

        let img_bytes = match self.client.get(cover_url).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) => b.to_vec(),
                Err(e) => {
                    warn!("Failed to read cover bytes from {cover_url}: {e}");
                    return None;
                }
            },
            Err(e) => {
                warn!("Failed to download cover from {cover_url}: {e}");
                return None;
            }
        };

        let img_bytes = Self::normalize_cover(&img_bytes).unwrap_or(img_bytes);

        let disk = self.disk_path(key);
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        if let Err(e) = fs::write(&disk, &img_bytes) {
            warn!("Failed to write cover to disk {disk:?}: {e}");
        }

        Some(CoverData {
            data: img_bytes,
            mime: "image/jpeg".to_string(),
        })
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            other => format!("%{:02X}", other as u8),
        })
        .collect()
}
