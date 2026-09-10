// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist file format abstraction (M3U8, PLS) shared by the daemon's import
// and export paths.
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

use crate::track::{Playlist, TrackInfo};

/// Supported on-disk playlist formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistFormatKind {
    M3u8,
    Pls,
}

impl PlaylistFormatKind {
    /// File-name extension (without leading dot) for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            PlaylistFormatKind::M3u8 => "m3u8",
            PlaylistFormatKind::Pls => "pls",
        }
    }

    /// Detect a format from a file name's extension, defaulting to M3U8.
    pub fn from_path(path: &str) -> Self {
        match std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("pls") => PlaylistFormatKind::Pls,
            _ => PlaylistFormatKind::M3u8,
        }
    }
}

/// A parser/serializer for a playlist file format. Each implementation turns
/// raw file content into an ordered list of track paths (import) or renders
/// a playlist + tracks back to file content (export).
pub trait PlaylistFormat {
    /// Extract the ordered track paths from `content`. Lines that cannot be
    /// resolved are omitted; the caller is responsible for path resolution
    /// against the import file's directory.
    fn parse_track_lines(&self, content: &str) -> Vec<String>;

    /// Render `tracks` under `playlist` into file content.
    fn render(&self, playlist: &Playlist, tracks: &[TrackInfo]) -> String;
}

/// M3U8 (`#EXTM3U` extended M3U) playlist format.
pub struct M3u8Format;

impl M3u8Format {
    fn file_name(name: &str) -> String {
        let cleaned: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        cleaned.trim().replace(' ', "_")
    }
}

impl PlaylistFormat for M3u8Format {
    fn parse_track_lines(&self, content: &str) -> Vec<String> {
        content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    fn render(&self, playlist: &Playlist, tracks: &[TrackInfo]) -> String {
        let mut lines = vec![
            "#EXTM3U".to_string(),
            format!("#PLAYLIST: {}", Self::file_name(&playlist.name)),
        ];
        for track in tracks {
            let dur_secs = track.duration as u64;
            lines.push(format!(
                "#EXTINF:{},{} - {}",
                dur_secs, track.artist, track.title
            ));
            lines.push(track.path.clone());
        }
        lines.join("\n")
    }
}

/// PLS playlist format (`[playlist]` INI-style section).
pub struct PlsFormat;

impl PlaylistFormat for PlsFormat {
    fn parse_track_lines(&self, content: &str) -> Vec<String> {
        let mut paths = Vec::new();
        // Read every `FileN=...` key in order of N (not file order).
        let mut entries: Vec<(u64, String)> = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("file") {
                // rest looks like "N=path" or "N=path with =". Split on first '='.
                if let Some(eq) = rest.find('=') {
                    let num: u64 = rest[..eq].trim().parse().unwrap_or(0);
                    let value = line[line.find('=').unwrap() + 1..].trim().to_string();
                    if !value.is_empty() {
                        entries.push((num, value));
                    }
                }
            }
        }
        entries.sort_by_key(|(n, _)| *n);
        for (_, path) in entries {
            paths.push(path);
        }
        paths
    }

    fn render(&self, playlist: &Playlist, tracks: &[TrackInfo]) -> String {
        let mut out = String::new();
        out.push_str("[playlist]\n");
        for (i, track) in tracks.iter().enumerate() {
            let n = i + 1;
            let dur_secs = track.duration.round() as u64;
            out.push_str(&format!("File{n}={}\n", track.path));
            out.push_str(&format!("Title{n}={} - {}\n", track.artist, track.title));
            out.push_str(&format!("Length{n}={dur_secs}\n"));
        }
        out.push_str(&format!("NumberOfEntries={}\n", tracks.len()));
        out.push_str(&format!("PlaylistName={}\n", playlist.name));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track(i: i64, path: &str) -> TrackInfo {
        let mut t = TrackInfo::default();
        t.id = i;
        t.path = path.to_string();
        t.title = format!("Title {i}");
        t.artist = "Artist".to_string();
        t.duration = 180.0;
        t
    }

    #[test]
    fn kind_from_path_and_extension() {
        assert_eq!(
            PlaylistFormatKind::from_path("mix.pls"),
            PlaylistFormatKind::Pls
        );
        assert_eq!(
            PlaylistFormatKind::from_path("mix.m3u8"),
            PlaylistFormatKind::M3u8
        );
        assert_eq!(
            PlaylistFormatKind::from_path("mix.txt"),
            PlaylistFormatKind::M3u8
        );
        assert_eq!(PlaylistFormatKind::M3u8.extension(), "m3u8");
        assert_eq!(PlaylistFormatKind::Pls.extension(), "pls");
    }

    #[test]
    fn m3u8_roundtrip() {
        let fmt = M3u8Format;
        let playlist = Playlist {
            id: 1,
            name: "My Mix".to_string(),
            created_at: "now".to_string(),
            track_count: 2,
        };
        let tracks = vec![
            sample_track(1, "/a/one.mp3"),
            sample_track(2, "/b/two.flac"),
        ];
        let content = fmt.render(&playlist, &tracks);
        let paths = fmt.parse_track_lines(&content);
        assert_eq!(
            paths,
            vec!["/a/one.mp3".to_string(), "/b/two.flac".to_string()]
        );
        assert!(content.starts_with("#EXTM3U"));
        assert!(content.contains("#EXTINF:180,Artist - Title 1"));
    }

    #[test]
    fn pls_roundtrip() {
        let fmt = PlsFormat;
        let playlist = Playlist {
            id: 1,
            name: "My Mix".to_string(),
            created_at: "now".to_string(),
            track_count: 2,
        };
        let tracks = vec![
            sample_track(1, "/a/one.mp3"),
            sample_track(2, "/b/two.flac"),
        ];
        let content = fmt.render(&playlist, &tracks);
        println!("{content}");
        let paths = fmt.parse_track_lines(&content);
        assert_eq!(
            paths,
            vec!["/a/one.mp3".to_string(), "/b/two.flac".to_string()]
        );
        assert!(content.contains("[playlist]"));
        assert!(content.contains("NumberOfEntries=2"));
    }

    #[test]
    fn pls_parses_out_of_order_and_skips_directives() {
        let fmt = PlsFormat;
        let content = "[playlist]\nFile2=/b/two.flac\nTitle2=x\nFile1=/a/one.mp3\nLength1=10\nNumberOfEntries=2\nPlaylistName=foo\n";
        assert_eq!(
            fmt.parse_track_lines(content),
            vec!["/a/one.mp3".to_string(), "/b/two.flac".to_string()]
        );
    }
}
