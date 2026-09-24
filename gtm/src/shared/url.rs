// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Shared URL parsing: host extraction and provider classification
//
// This is free software released under the GPL-3.0 license.

/// Host of a URL, lowercased, without a leading `www.`. Falls back to manual
/// splitting for inputs `reqwest::Url` cannot parse (yt-dlp gets raw user
/// input), so a malformed URL still classifies by its leading host token.
pub fn host(url: &str) -> String {
    let raw = match reqwest::Url::parse(url) {
        Ok(u) => u.host_str().unwrap_or_default().to_string(),
        Err(_) => url
            .split_once("://")
            .map_or(url, |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string(),
    };
    raw.trim_start_matches("www.").to_ascii_lowercase()
}

/// Whether a URL points at YouTube, including its CDN hosts. The client's
/// live-stream classifier and the daemon's path parser must agree, otherwise
/// a googlevideo URL is treated as an endless radio stream.
pub fn is_youtube(url: &str) -> bool {
    let host = host(url);
    host.contains("youtube") || host.contains("youtu.be") || host.contains("googlevideo")
}

/// Provider label for URLs handled by yt-dlp's extractors. Only these known
/// hosts are yt-dlp input; every other http URL stays a plain stream.
pub fn ytdlp_label(url: &str) -> Option<&'static str> {
    let host = host(url);
    if host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be" {
        return Some("YouTube");
    }
    if host == "bandcamp.com" || host.ends_with(".bandcamp.com") {
        return Some("Bandcamp");
    }
    match host.as_str() {
        "soundcloud.com" => Some("SoundCloud"),
        "mixcloud.com" => Some("Mixcloud"),
        "music.163.com" => Some("Netease"),
        "bilibili.com" | "m.bilibili.com" => Some("Bilibili"),
        "vimeo.com" | "player.vimeo.com" => Some("Vimeo"),
        "twitch.tv" => Some("Twitch"),
        "audiomack.com" => Some("Audiomack"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{host, is_youtube, ytdlp_label};

    #[test]
    fn host_strips_scheme_www_and_case() {
        assert_eq!(host("https://WWW.YouTube.com/watch?v=x"), "youtube.com");
        assert_eq!(host("http://soundcloud.com/user/track"), "soundcloud.com");
        assert_eq!(host("soundcloud.com/x"), "soundcloud.com");
    }

    #[test]
    fn host_keeps_port_and_subdomain() {
        assert_eq!(
            host("http://Music.Example.COM:8080/a"),
            "music.example.com:8080"
        );
        assert_eq!(host("https://m.bilibili.com/x"), "m.bilibili.com");
    }

    #[test]
    fn youtube_detects_cdn() {
        assert!(is_youtube("https://rr1.googlevideo.com/videoplayback?x=1"));
        assert!(is_youtube("https://youtu.be/abc"));
        assert!(!is_youtube("https://stream.example.com/radio.mp3"));
    }

    #[test]
    fn labels_map_known_hosts() {
        assert_eq!(
            ytdlp_label("https://music.bandcamp.com/a"),
            Some("Bandcamp")
        );
        assert_eq!(ytdlp_label("https://m.bilibili.com/x"), Some("Bilibili"));
        assert_eq!(ytdlp_label("https://example.com/a"), None);
    }
}
