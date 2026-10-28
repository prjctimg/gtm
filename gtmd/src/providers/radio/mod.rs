// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Radio Browser directory client: search + top stations, native streaming
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use gtm::shared::radio::{RadioCountry, RadioStation, RadioTag, cliamp_slug, cliamp_station_id};

/// Radio Browser API mirrors. `all.api.radio-browser.info` round-robins across
/// the public servers; the per-region mirrors are used as fallback hosts.
const HOSTS: [&str; 3] = [
    "https://all.api.radio-browser.info",
    "https://de1.api.radio-browser.info",
    "https://nl1.api.radio-browser.info",
];

const CLIENT_NAME: &str = "gtm";

/// The cliamp radio channel directory: a small, curated set of live streams.
const CLIAMP_STATIONS: &str = "https://radio.cliamp.stream/stations";

/// Directory client. Radio Browser searches return stations whose
/// `url_resolved` stream is played natively over HTTP; cliamp returns its own
/// curated channels. Both use the same native stream path.
pub struct RadioBrowserManager {
    client: reqwest::Client,
    /// The cliamp channel list, fetched once per session or on refresh.
    ///
    /// Resolution runs on every reconnect and on the tracklist tick, so
    /// re-reading a static directory for each of those would be needless
    /// traffic for a list that changes with the server's own deployment.
    cliamp_cache: std::sync::Mutex<Option<Vec<RadioStation>>>,
}

impl RadioBrowserManager {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(format!(
                    "gtm/{} ({})",
                    env!("CARGO_PKG_VERSION"),
                    CLIENT_NAME
                ))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap_or_default(),
            cliamp_cache: std::sync::Mutex::new(None),
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

    /// The cliamp radio channel directory.
    ///
    /// This is a separate directory from Radio Browser: its ids are slugs such
    /// as `lofi`, its streams are all served by the same host, and some
    /// channels also expose a song catalogue. The catalogue is intentionally
    /// not read here: its list is the station's library, not its recent
    /// history, while the live stream carries the authoritative ICY title.
    pub async fn cliamp(&self, refresh: bool) -> Result<Vec<RadioStation>, String> {
        if !refresh
            && let Some(cached) = self
                .cliamp_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        {
            return Ok(cached);
        }
        let resp = self
            .client
            .get(CLIAMP_STATIONS)
            .send()
            .await
            .map_err(|e| format!("cliamp: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("cliamp: HTTP {}", resp.status()));
        }
        let body = resp.text().await.map_err(|e| format!("cliamp body: {e}"))?;
        let stations = parse_cliamp_stations(&body);
        if stations.is_empty() {
            return Err("cliamp returned no playable channels".to_string());
        }
        *self.cliamp_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(stations.clone());
        Ok(stations)
    }

    /// Look up a single station by its directory id, so a `radio://` queue
    /// item can be re-resolved to a playable URL on replay / next.
    pub async fn by_uuid(&self, uuid: &str) -> Result<RadioStation, String> {
        if cliamp_slug(uuid).is_some() {
            return self
                .cliamp(false)
                .await?
                .into_iter()
                .find(|station| station.id == uuid)
                .ok_or_else(|| format!("cliamp channel {uuid} not found"));
        }
        let resp = self
            .get(
                &format!("/json/stations/byuuid/{uuid}"),
                &[("hidebroken", "true")],
            )
            .await
            .map_err(|e| format!("lookup: {e}"))?;
        let mut stations = parse_stations(&resp);
        stations
            .pop()
            .ok_or_else(|| "station not found".to_string())
    }

    /// The most-used directory tags (`/json/tags`).
    pub async fn tags(&self, limit: u16) -> Result<Vec<RadioTag>, String> {
        let resp = self
            .get(
                "/json/tags",
                &[("limit", &limit.to_string()), ("hidebroken", "true")],
            )
            .await
            .map_err(|e| format!("tags: {e}"))?;
        Ok(parse_tags(&resp))
    }

    /// The most-used directory countries (`/json/countries`).
    pub async fn countries(&self, limit: u16) -> Result<Vec<RadioCountry>, String> {
        let resp = self
            .get(
                "/json/countries",
                &[("limit", &limit.to_string()), ("hidebroken", "true")],
            )
            .await
            .map_err(|e| format!("countries: {e}"))?;
        Ok(parse_countries(&resp))
    }

    /// Stations carrying a given directory tag. `tag` is URL-encoded by the
    /// query builder, so values like "80s" or "latin jazz" are safe here.
    pub async fn stations_by_tag(
        &self,
        tag: &str,
        limit: u16,
    ) -> Result<Vec<RadioStation>, String> {
        let resp = self
            .get(
                "/json/stations/search",
                &[
                    ("tag", tag),
                    ("limit", &limit.to_string()),
                    ("hidebroken", "true"),
                ],
            )
            .await
            .map_err(|e| format!("by-tag: {e}"))?;
        Ok(parse_stations(&resp))
    }

    /// Stations from a given directory country.
    pub async fn stations_by_country(
        &self,
        country: &str,
        limit: u16,
    ) -> Result<Vec<RadioStation>, String> {
        let resp = self
            .get(
                "/json/stations/search",
                &[
                    ("country", country),
                    ("limit", &limit.to_string()),
                    ("hidebroken", "true"),
                ],
            )
            .await
            .map_err(|e| format!("by-country: {e}"))?;
        Ok(parse_stations(&resp))
    }

    async fn get(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut last_err = "no radio-browser server reachable".to_string();
        for host in HOSTS {
            let mut url =
                reqwest::Url::parse(&format!("{host}{path}")).map_err(|e| format!("url: {e}"))?;
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

impl Default for RadioBrowserManager {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_stations(items: &[serde_json::Value]) -> Vec<RadioStation> {
    items
        .iter()
        .filter_map(|s| {
            let id = s.get("stationuuid").and_then(|v| v.as_str())?;
            // Only offer streams that recently passed a working check when the
            // directory classifies them.
            if let Some(_ok) = s
                .get("lastcheckok")
                .and_then(|v| v.as_i64())
                .filter(|&ok| ok != 1)
            {
                return None;
            }
            Some(RadioStation {
                id: id.to_string(),
                name: s
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Station")
                    .to_string(),
                homepage: s
                    .get("homepage")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                url: s
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                url_resolved: s
                    .get("url_resolved")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                country: s
                    .get("country")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                language: s
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tags: s
                    .get("tags")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                codec: s
                    .get("codec")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                bitrate_kbps: s.get("bitrate").and_then(|v| v.as_u64()),
                votes: s.get("votes").and_then(|v| v.as_u64()).unwrap_or(0),
                favicon: s
                    .get("favicon")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                description: String::new(),
            })
        })
        .collect()
}

/// Parse cliamp's channel directory.
///
/// A row is only offered when it has a slug that is safe inside the `radio://`
/// path, a name or slug to label it, and an HTTP(S) stream. The directory is
/// public and untrusted: a malformed row should disappear, not turn into a
/// station that cannot be played.
fn parse_cliamp_stations(body: &str) -> Vec<RadioStation> {
    #[derive(serde::Deserialize)]
    struct Directory {
        #[serde(default)]
        stations: Vec<Channel>,
    }
    #[derive(serde::Deserialize)]
    struct Channel {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        genre: String,
        #[serde(default)]
        stream: String,
    }

    let Ok(directory) = serde_json::from_str::<Directory>(body) else {
        return Vec::new();
    };
    directory
        .stations
        .into_iter()
        .filter_map(|channel| {
            let slug = channel.id.trim();
            if slug.is_empty()
                || !slug
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return None;
            }
            let stream = channel.stream.trim();
            let url = reqwest::Url::parse(stream).ok()?;
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            let name = match channel.name.trim() {
                "" => slug,
                name => name,
            };
            Some(RadioStation {
                id: cliamp_station_id(slug),
                name: name.to_string(),
                homepage: "https://www.cliamp.stream/".to_string(),
                url: stream.to_string(),
                url_resolved: stream.to_string(),
                country: String::new(),
                language: String::new(),
                tags: channel.genre.trim().to_string(),
                codec: "MP3".to_string(),
                bitrate_kbps: None,
                votes: 0,
                favicon: String::new(),
                description: channel.description.trim().to_string(),
            })
        })
        .collect()
}

fn parse_tags(items: &[serde_json::Value]) -> Vec<RadioTag> {
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name").and_then(|v| v.as_str())?;
            Some(RadioTag {
                name: name.to_string(),
                station_count: s.get("stationcount").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

fn parse_countries(items: &[serde_json::Value]) -> Vec<RadioCountry> {
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name").and_then(|v| v.as_str())?;
            Some(RadioCountry {
                name: name.to_string(),
                station_count: s.get("stationcount").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cliamp_rows_are_namespaced_and_playable() {
        let stations = parse_cliamp_stations(
            r#"{"stations":[
                {"id":"lofi","name":"Lofi","description":"Beats","genre":"Lofi",
                 "stream":"https://radio.cliamp.stream/lofi/stream"},
                {"id":"bad/id","name":"Bad","stream":"https://radio.cliamp.stream/bad/stream"},
                {"id":"ftp","name":"FTP","stream":"ftp://radio.cliamp.stream/ftp/stream"},
                {"id":"","name":"No id","stream":"https://radio.cliamp.stream/none/stream"}
            ]}"#,
        );
        assert_eq!(stations.len(), 1);
        let station = &stations[0];
        assert_eq!(station.id, "cliamp:lofi");
        assert_eq!(station.name, "Lofi");
        assert_eq!(station.tags, "Lofi");
        assert_eq!(station.description, "Beats");
        assert_eq!(
            station.playable_url(),
            "https://radio.cliamp.stream/lofi/stream"
        );
        assert_eq!(cliamp_slug(&station.id), Some("lofi"));
    }

    #[test]
    fn an_invalid_cliamp_document_is_empty() {
        assert!(parse_cliamp_stations("not json").is_empty());
        assert!(parse_cliamp_stations("{}").is_empty());
    }
}

pub mod tracklist;
