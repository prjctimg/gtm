// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// User-extensible yt-dlp search hosts for the daemon's YT-family search.
//
// This is free software released under the GPL-3.0 license.

/// One yt-dlp search host: the query prefix the YT search picker recognizes
/// (e.g. `scsearch:`), the yt-dlp search-extractor prefix it maps to (e.g.
/// `scsearch10`), and the site name shown in the picker hint line.
///
/// The table is data-driven so users can extend the supported streaming
/// providers without code changes: every entry is just a prefix → yt-dlp
/// extractor mapping, and adding a row is enough for a new site to flow
/// through the existing search/resolve/play pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtSearchHost {
    /// Query prefix typed in the search picker / CLI, e.g. `"scsearch:"`.
    pub prefix: String,
    /// yt-dlp search extractor the prefix maps to, e.g. `"scsearch10"`.
    pub extractor: String,
    /// Display name shown in the picker hint, e.g. `"SoundCloud"`.
    pub name: String,
}

impl YtSearchHost {
    pub fn new(
        prefix: impl Into<String>,
        extractor: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            extractor: extractor.into(),
            name: name.into(),
        }
    }
}

/// The built-in hosts (cliamp-compatible prefixes). Users can extend or
/// rename them via `GTM_YT_HOSTS` (see [`yt_hosts`]).
pub fn default_yt_hosts() -> Vec<YtSearchHost> {
    vec![
        YtSearchHost::new("scsearch:", "scsearch10", "SoundCloud"),
        YtSearchHost::new("bilisearch:", "bilisearch10", "Bilibili"),
        YtSearchHost::new("mcsearch:", "mcsearch10", "Mixcloud"),
        YtSearchHost::new("ytsearch10:", "ytsearch10", "YouTube"),
        YtSearchHost::new("ytsearch:", "ytsearch10", "YouTube Music"),
    ]
}

/// Full host table: [`default_yt_hosts`] plus `GTM_YT_HOSTS` additions.
///
/// `GTM_YT_HOSTS` is a comma-separated list of `prefix|extractor|Name` rows,
/// e.g. `GTM_YT_HOSTS="vksearch:|vksearch10|VK,vimeosearch:|vimeosearch10|Vimeo"`.
/// A row whose prefix matches a built-in host replaces it; any other row is
/// appended. Because the TUI spawns the daemon with an inherited environment,
/// one exported variable covers both the picker hints and the daemon's search
/// routing.
pub fn yt_hosts() -> Vec<YtSearchHost> {
    let mut hosts = default_yt_hosts();
    let Some(raw) = std::env::var("GTM_YT_HOSTS").ok() else {
        return hosts;
    };
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.splitn(3, '|');
        let (Some(prefix), Some(extractor), Some(name)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (prefix, extractor, name) = (prefix.trim(), extractor.trim(), name.trim());
        if prefix.is_empty() || extractor.is_empty() || name.is_empty() {
            continue;
        }
        match hosts.iter_mut().find(|h| h.prefix == prefix) {
            Some(existing) => {
                existing.extractor = extractor.to_string();
                existing.name = name.to_string();
            }
            None => hosts.push(YtSearchHost::new(prefix, extractor, name)),
        }
    }
    hosts
}

/// The first host whose prefix matches `query`, returning the stripped query
/// remainder and the host. Longest-prefix-first ordering in the table matters
/// for overlapping prefixes (`ytsearch10:` before `ytsearch:`).
pub fn match_yt_host<'a>(
    query: &'a str,
    hosts: &'a [YtSearchHost],
) -> Option<(&'a str, &'a YtSearchHost)> {
    hosts.iter().find_map(|h| {
        query
            .strip_prefix(&h.prefix)
            .filter(|rest| !rest.trim().is_empty())
            .map(|rest| (rest.trim(), h))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_builtin_prefixes() {
        let hosts = default_yt_hosts();
        assert!(hosts.iter().any(|h| h.prefix == "scsearch:"));
        assert!(hosts.iter().any(|h| h.prefix == "bilisearch:"));
        assert!(hosts.iter().any(|h| h.prefix == "mcsearch:"));
        assert!(hosts.iter().any(|h| h.prefix == "ytsearch:"));
        // Overlapping prefixes must keep longest-first order so an exact
        // `ytsearch10:` query never matches the shorter `ytsearch:` row.
        let yt = hosts
            .iter()
            .position(|h| h.prefix == "ytsearch10:")
            .unwrap();
        let yt_short = hosts.iter().position(|h| h.prefix == "ytsearch:").unwrap();
        assert!(yt < yt_short);
    }

    #[test]
    fn match_requires_a_real_query() {
        let hosts = default_yt_hosts();
        assert!(match_yt_host("scsearch:", &hosts).is_none());
        let (rest, host) = match_yt_host("scsearch:  the hills  ", &hosts).unwrap();
        assert_eq!(rest, "the hills");
        assert_eq!(host.name, "SoundCloud");
    }

    #[test]
    fn overlapping_prefixes_match_longest_first() {
        let hosts = default_yt_hosts();
        let (_, host) = match_yt_host("ytsearch10:foo", &hosts).unwrap();
        assert_eq!(host.name, "YouTube");
        let (_, host) = match_yt_host("ytsearch:foo", &hosts).unwrap();
        assert_eq!(host.name, "YouTube Music");
    }

    #[test]
    fn custom_hosts_append_and_override() {
        let mut hosts = default_yt_hosts();
        // Simulates the GTM_YT_HOSTS override logic applied to defaults.
        hosts.push(YtSearchHost::new("vksearch:", "vksearch10", "VK"));
        let (_, host) = match_yt_host("vksearch:ремикс", &hosts).unwrap();
        assert_eq!(host.extractor, "vksearch10");
        // Unknown prefixes still match nothing.
        assert!(match_yt_host("twitch:foo", &hosts).is_none());
    }
}
