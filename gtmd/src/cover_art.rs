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
const DEEZER_API: &str = "https://api.deezer.com/search";
const RATE_LIMIT_MS: u64 = 200;

#[derive(Debug, Clone)]
pub struct CoverData {
    pub data: Vec<u8>,
    pub mime: String,
}

pub struct CoverCache {
    memory: Arc<Mutex<LruCache<String, CoverData>>>,
    cache_dir: PathBuf,
    client: Client,
}

impl CoverCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(cache_dir.join("covers")).ok();
        Self {
            memory: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_SIZE).unwrap(),
            ))),
            cache_dir,
            client: Client::new(),
        }
    }

    pub fn cache_key(artist: &str, album: &str) -> String {
        let mut h = Sha256::new();
        h.update(format!("{}:{}", artist, album).as_bytes());
        hex::encode(&h.finalize()[..8])
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join("covers").join(format!("{key}.jpg"))
    }

    pub async fn get_cover(&mut self, artist: &str, album: &str) -> Option<CoverData> {
        let artist = if artist.is_empty() { "Unknown Artist" } else { artist };
        let album = if album.is_empty() { "Unknown Album" } else { album };
        let key = Self::cache_key(artist, album);

        {
            let mut mem = self.memory.lock().await;
            if let Some(c) = mem.get(&key) {
                return Some(c.clone());
            }
        }

        let disk = self.disk_path(&key);
        if disk.exists() {
            if let Ok(data) = fs::read(&disk) {
                let cd = CoverData {
                    mime: "image/jpeg".to_string(),
                    data,
                };
                let mut mem = self.memory.lock().await;
                mem.put(key, cd.clone());
                return Some(cd);
            }
        }

        let cd = self.fetch_from_deezer(artist, album, &key).await;
        if let Some(ref cd) = cd {
            let mut mem = self.memory.lock().await;
            mem.put(key, cd.clone());
        }
        cd
    }

    async fn fetch_from_deezer(&self, artist: &str, album: &str, key: &str) -> Option<CoverData> {
        let query = format!(
            "artist:\"{}\" album:\"{}\"",
            urlencoding(artist),
            urlencoding(album)
        );

        tokio::time::sleep(std::time::Duration::from_millis(RATE_LIMIT_MS)).await;

        let resp = match self.client.get(DEEZER_API).query(&[("q", &query)]).send().await {
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
