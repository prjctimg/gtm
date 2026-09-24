// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Cover art fetching from Deezer API with disk and LRU cache
//
// This is free software released under the GPL-3.0 license.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use tokio::sync::Mutex;
use tracing::warn;

use crate::deezer::DeezerSearch;
use crate::musicbrainz::MusicBrainz;

/// In-memory entry budget for album covers. Each entry is a normalized
/// 500x500 JPEG (~20-150 KB), so 64 entries bind memory to roughly 8 MB worst
/// case regardless of the entry cap below.
const MEMORY_BUDGET_BYTES: usize = 8 * 1024 * 1024;
/// In-memory entry budget for artist images (often larger PNGs).
const ARTIST_MEMORY_BUDGET_BYTES: usize = 2 * 1024 * 1024;
const CACHE_SIZE: usize = 64;
const ARTIST_CACHE_SIZE: usize = 30;
const DEEZER_API: &str = "https://api.deezer.com/search";
const RATE_LIMIT_MS: u64 = 200;
const MIN_COVER_DIM: u32 = 300;
/// Default upper bound for the on-disk cover caches. User-configurable via
/// `cover_cache_mb`; oldest-by-mtime files are pruned once the combined size of
/// `covers/` and `artist_covers/` exceeds it.
pub const DISK_CACHE_DEFAULT: u64 = 512 * 1024 * 1024;

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
    memory_bytes: AtomicUsize,
    artist_memory_bytes: AtomicUsize,
    cache_dir: PathBuf,
    client: Client,
    /// Number of covers written to disk; used to throttle the size-prune walk.
    disk_writes: AtomicUsize,
    /// Combined on-disk budget for `covers/` and `artist_covers/`.
    disk_cap: AtomicU64,
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
            memory_bytes: AtomicUsize::new(0),
            artist_memory_bytes: AtomicUsize::new(0),
            cache_dir,
            client: Client::new(),
            disk_writes: AtomicUsize::new(0),
            disk_cap: AtomicU64::new(DISK_CACHE_DEFAULT),
        }
    }

    /// Change the on-disk budget and prune immediately if already over it.
    pub fn set_disk_cap(&self, cap: u64) {
        self.disk_cap.store(cap.max(1), Ordering::Relaxed);
    }

    /// Current on-disk budget in bytes.
    pub fn disk_cap(&self) -> u64 {
        self.disk_cap.load(Ordering::Relaxed)
    }

    /// Drop every in-memory entry. Used by the explicit cache clear so a
    /// "cleared" cache really is empty without waiting for eviction.
    pub async fn clear_mem(&self) {
        self.memory.lock().await.clear();
        self.artist_memory.lock().await.clear();
        self.memory_bytes.store(0, Ordering::Relaxed);
        self.artist_memory_bytes.store(0, Ordering::Relaxed);
    }

    /// Bytes currently held on disk by both cover directories.
    pub async fn disk_use(&self) -> u64 {
        self.disk_size(&self.cache_dir.join("covers")).await
            + self.disk_size(&self.cache_dir.join("artist_covers")).await
    }

    /// Insert `cd` into the LRU, evicting least-recently-used entries once the
    /// in-memory byte budget is exceeded. The evicted entry has its byte cost
    /// refunded from the counter of record.
    fn insert_mem(
        cache: &mut LruCache<String, CoverData>,
        counter: &AtomicUsize,
        budget: usize,
        key: String,
        cd: CoverData,
    ) {
        let entry_size = cd.data.len() + cd.mime.len();
        if let Some((_, evicted)) = cache.push(key, cd) {
            counter.fetch_sub(evicted.data.len() + evicted.mime.len(), Ordering::Relaxed);
        }
        counter.fetch_add(entry_size, Ordering::Relaxed);
        while counter.load(Ordering::Relaxed) > budget {
            match cache.pop_lru() {
                Some((_, evicted)) => {
                    counter.fetch_sub(evicted.data.len() + evicted.mime.len(), Ordering::Relaxed);
                }
                None => break,
            }
        }
    }

    /// Total bytes currently held by the in-memory cover caches.
    pub fn memory_use(&self) -> usize {
        self.memory_bytes.load(Ordering::Relaxed) + self.artist_memory_bytes.load(Ordering::Relaxed)
    }

    /// Write cover bytes to a disk cache directory, periodically pruning
    /// oldest-by-mtime files once the combined cache exceeds the budget.
    async fn store_disk(&self, path: &PathBuf, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        if let Err(e) = tokio::fs::write(path, bytes).await {
            warn!("Failed to write cover to disk {path:?}: {e}");
        }
        if self
            .disk_writes
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(8)
        {
            self.prune_disk_cache().await;
        }
    }

    async fn disk_size(&self, dir: &std::path::Path) -> u64 {
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return 0;
        };
        let mut total = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(md) = entry.metadata().await
                && md.is_file()
            {
                total += md.len();
            }
        }
        total
    }

    /// Prune oldest-by-mtime files across both cover directories until the
    /// total fits the configured budget. Artist images are pruned too, so the
    /// cache cannot grow past the cap.
    pub async fn prune_disk_cache(&self) {
        let cap = self.disk_cap();
        let mut files: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
        let mut total = 0u64;
        for name in ["covers", "artist_covers"] {
            let dir = self.cache_dir.join(name);
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(md) = entry.metadata().await
                    && md.is_file()
                {
                    total += md.len();
                    files.push((md.modified().unwrap_or(UNIX_EPOCH), entry.path(), md.len()));
                }
            }
        }
        if total <= cap {
            return;
        }
        files.sort_by_key(|(mtime, _, _)| *mtime);
        for (_, path, len) in files {
            if total <= cap {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    pub fn too_small(data: &[u8]) -> bool {
        match image::load_from_memory(data) {
            Ok(img) => img.width() < MIN_COVER_DIM || img.height() < MIN_COVER_DIM,
            Err(_) => true,
        }
    }

    /// Normalize arbitrary album-cover bytes to a single standard size for the
    /// whole app: centre-cropped (never distorted) 500x500 JPEG at quality 90.
    /// Returns `None` when the input can't be decoded so callers can keep the
    /// original bytes.
    pub fn normalize(data: &[u8]) -> Option<Vec<u8>> {
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
    pub async fn put(&self, artist: &str, album: &str, bytes: Vec<u8>) {
        if bytes.is_empty() || Self::too_small(&bytes) {
            return;
        }
        let bytes = Self::normalize(&bytes).unwrap_or(bytes);
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
        self.store_disk(&disk, &bytes).await;
        let mut mem = self.memory.lock().await;
        Self::insert_mem(&mut mem, &self.memory_bytes, MEMORY_BUDGET_BYTES, key, cd);
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
        self.store_disk(&disk, &bytes).await;
        let mut mem = self.artist_memory.lock().await;
        Self::insert_mem(
            &mut mem,
            &self.artist_memory_bytes,
            ARTIST_MEMORY_BUDGET_BYTES,
            key,
            cd,
        );
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join("covers").join(format!("{key}.jpg"))
    }

    /// Stable key for a remote image URL, so provider artwork browsed in a
    /// picker is persisted instead of re-downloaded on every visit.
    pub fn url_key(url: &str) -> String {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        format!("u{}", hex::encode(&h.finalize()[..8]))
    }

    /// Return cached bytes for `url`, fetching and persisting them on a miss.
    /// `fetch` is only called when neither memory nor disk has the image.
    pub async fn get_url<F, Fut>(&self, url: &str, fetch: F) -> Option<CoverData>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Option<Vec<u8>>>,
    {
        if url.trim().is_empty() {
            return None;
        }
        let key = Self::url_key(url);
        {
            let mut mem = self.memory.lock().await;
            if let Some(c) = mem.get(&key) {
                return Some(c.clone());
            }
        }
        let disk = self.disk_path(&key);
        if let Ok(data) = tokio::fs::read(&disk).await
            && !Self::too_small(&data)
        {
            let cd = CoverData {
                mime: "image/jpeg".to_string(),
                data,
            };
            let mut mem = self.memory.lock().await;
            Self::insert_mem(
                &mut mem,
                &self.memory_bytes,
                MEMORY_BUDGET_BYTES,
                key,
                cd.clone(),
            );
            return Some(cd);
        }
        let raw = fetch().await?;
        let data = Self::normalize(&raw).unwrap_or(raw);
        let cd = CoverData {
            mime: "image/jpeg".to_string(),
            data: data.clone(),
        };
        self.store_disk(&disk, &data).await;
        let mut mem = self.memory.lock().await;
        Self::insert_mem(
            &mut mem,
            &self.memory_bytes,
            MEMORY_BUDGET_BYTES,
            key,
            cd.clone(),
        );
        Some(cd)
    }

    pub async fn get(
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
                && !Self::too_small(&c.data)
            {
                return Some(c.clone());
            }
        }

        let disk = self.disk_path(&key);
        if disk.exists()
            && let Ok(data) = tokio::fs::read(&disk).await
        {
            if !Self::too_small(&data) {
                let cd = CoverData {
                    mime: "image/jpeg".to_string(),
                    data,
                };
                let mut mem = self.memory.lock().await;
                Self::insert_mem(
                    &mut mem,
                    &self.memory_bytes,
                    MEMORY_BUDGET_BYTES,
                    key,
                    cd.clone(),
                );
                return Some(cd);
            }
            // Small cover on disk: remove it and re-fetch from Deezer
            tokio::fs::remove_file(&disk).await.ok();
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
            Self::insert_mem(
                &mut mem,
                &self.memory_bytes,
                MEMORY_BUDGET_BYTES,
                key,
                cd.clone(),
            );
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
        let mb = MusicBrainz::new();
        let found = mb.find_album(artist, album).await.ok().flatten()?;
        let bytes = mb.download_cover(&found.release_group_id).await?;
        let bytes = Self::normalize(&bytes).unwrap_or(bytes);
        let disk = self.disk_path(key);
        self.store_disk(&disk, &bytes).await;
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
            && let Ok(data) = tokio::fs::read(&disk).await
        {
            let cd = CoverData {
                mime: "image/jpeg".to_string(),
                data,
            };
            let mut mem = self.artist_memory.lock().await;
            Self::insert_mem(
                &mut mem,
                &self.artist_memory_bytes,
                ARTIST_MEMORY_BUDGET_BYTES,
                key,
                cd.clone(),
            );
            return Some(cd);
        }
        let deezer = DeezerSearch::new();
        let img_bytes = deezer.artist_image(artist).await?;
        // Normalize before caching so the `.jpg` on disk always matches the
        // `image/jpeg` mime, instead of storing raw PNG bytes under a jpg name.
        let img_bytes = Self::normalize(&img_bytes).unwrap_or(img_bytes);
        let cd = CoverData {
            data: img_bytes.clone(),
            mime: "image/jpeg".to_string(),
        };
        self.store_disk(&disk, &img_bytes).await;
        let mut mem = self.artist_memory.lock().await;
        Self::insert_mem(
            &mut mem,
            &self.artist_memory_bytes,
            ARTIST_MEMORY_BUDGET_BYTES,
            key,
            cd.clone(),
        );
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

        let img_bytes = Self::normalize(&img_bytes).unwrap_or(img_bytes);

        let disk = self.disk_path(key);
        if let Some(parent) = disk.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        if let Err(e) = tokio::fs::write(&disk, &img_bytes).await {
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
