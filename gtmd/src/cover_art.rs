use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use lru::LruCache;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use tokio::sync::Mutex;

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

    fn cache_key(artist: &str, album: &str) -> String {
        let mut h = Sha256::new();
        h.update(format!("{}:{}", artist, album).as_bytes());
        hex::encode(&h.finalize()[..8])
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        self.cache_dir.join("covers").join(format!("{key}.jpg"))
    }

    pub async fn get_cover(&mut self, artist: &str, album: &str) -> Option<CoverData> {
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

        let resp = self
            .client
            .get(DEEZER_API)
            .query(&[("q", &query)])
            .send()
            .await
            .ok()?;

        let json: serde_json::Value = resp.json().await.ok()?;
        let data = json.get("data")?.as_array()?;
        let first = data.first()?;

        let cover_url = first
            .get("cover_big")
            .or_else(|| first.get("cover_medium"))?
            .as_str()?;

        let img_bytes = self.client.get(cover_url).send().await.ok()?.bytes().await.ok()?;

        let disk = self.disk_path(key);
        if let Some(parent) = disk.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&disk, &img_bytes).ok();

        Some(CoverData {
            data: img_bytes.to_vec(),
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
