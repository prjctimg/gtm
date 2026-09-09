// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Radio Browser directory client: search + top stations, native streaming
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use gtm_core::radio::RadioStation;

/// Radio Browser API mirrors. `all.api.radio-browser.info` round-robins across
/// the public servers; the per-region mirrors are used as fallback hosts.
const HOSTS: [&str; 3] = [
    "https://all.api.radio-browser.info",
    "https://de1.api.radio-browser.info",
    "https://nl1.api.radio-browser.info",
];

const CLIENT_NAME: &str = "gtm";

/// Stateless Radio Browser directory client. Searches return stations whose
/// `url_resolved` stream is played natively over HTTP.
pub struct RadioBrowserManager {
    client: reqwest::Client,
}

impl RadioBrowserManager {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(format!("gtm/{} ({})", env!("CARGO_PKG_VERSION"), CLIENT_NAME))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Search stations by name, keeping only verified-working streams.
    pub async fn search(&self, query: &str, limit: u16) -> Result<Vec<RadioStation>, String> {
        let params = [
            ("name", query),
            ("limit", &limit.to_string()),
            ("hidebroken", "true"),
        ];
        let resp = self
            .get("/json/stations/search", &params)
            .await
            .map_err(|e| format!("search: {e}"))?;
        Ok(parse_stations(&resp))
    }

    /// The most-voted working stations. Whole genres like "jazz" or "rock"
    /// can be narrowed further with `search`.
    pub async fn top(&self, limit: u16) -> Result<Vec<RadioStation>, String> {
        let resp = self
            .get(
                &format!("/json/stations/topvote/{limit}"),
                &[("hidebroken", "true")],
            )
            .await
            .map_err(|e| format!("top: {e}"))?;
        Ok(parse_stations(&resp))
    }

    /// Look up a single station by its `stationuuid`, so a `radio://` queue
    /// item can be re-resolved to a playable URL on replay / next.
    pub async fn by_uuid(&self, uuid: &str) -> Result<RadioStation, String> {
        let resp = self
            .get(&format!("/json/stations/byuuid/{uuid}"), &[("hidebroken", "true")])
            .await
            .map_err(|e| format!("lookup: {e}"))?;
        let mut stations = parse_stations(&resp);
        stations.pop().ok_or_else(|| "station not found".to_string())
    }

    async fn get<'a>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut last_err = "no radio-browser server reachable".to_string();
        for host in HOSTS {
            let mut url = reqwest::Url::parse(&format!("{host}{path}"))
                .map_err(|e| format!("url: {e}"))?;
            url.query_pairs_mut()
                .extend_pairs(params.iter().map(|(k, v)| (*k, *v)));
            match self.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return resp
                        .json::<Vec<serde_json::Value>>()
                        .await
                        .map_err(|e| format!("bad JSON: {e}"));
                }
                Ok(resp) => {
                    last_err = format!("HTTP {}", resp.status());
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(last_err)
    }
}

fn parse_stations(items: &[serde_json::Value]) -> Vec<RadioStation> {
    items
        .iter()
        .filter_map(|s| {
            let id = s.get("stationuuid").and_then(|v| v.as_str())?;
            // Only offer streams that recently passed a working check when the
            // directory classifies them.
            if let Some(ok) = s.get("lastcheckok").and_then(|v| v.as_i64()) {
                if ok != 1 {
                    return None;
                }
            }
            Some(RadioStation {
                id: id.to_string(),
                name: s.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Station").to_string(),
                homepage: s.get("homepage").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                url: s.get("url").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                url_resolved: s.get("url_resolved").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                country: s.get("country").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                language: s.get("language").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                tags: s.get("tags").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                codec: s.get("codec").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                bitrate_kbps: s.get("bitrate").and_then(|v| v.as_u64()),
                votes: s.get("votes").and_then(|v| v.as_u64()).unwrap_or(0),
                favicon: s.get("favicon").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            })
        })
        .collect()
}