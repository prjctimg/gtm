// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Library management: audio file scanning, metadata extraction, and persistence
//
// This is free software released under the GPL-3.0 license.

use std::fs;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::StandardVisualKey;
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::Timestamp;
use tracing::warn;

use gtm_core::track::{Playlist, TrackInfo};
use gtm_core::MetadataPatch;

const DB_NAME: &str = "library.db";

pub struct Library {
    conn: Connection,
    _watch_dirs: Mutex<Vec<String>>,
}

impl Library {
    pub fn new(db_dir: &str) -> Result<Self, String> {
        let path = format!("{}/{}", db_dir, DB_NAME);
        let conn = Connection::open(&path).map_err(|e| format!("db open: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                path         TEXT NOT NULL UNIQUE,
                title        TEXT NOT NULL DEFAULT '',
                artist       TEXT NOT NULL DEFAULT '',
                album        TEXT NOT NULL DEFAULT '',
                duration     REAL NOT NULL DEFAULT 0.0,
                track_number INTEGER,
                genre        TEXT NOT NULL DEFAULT '',
                year         INTEGER,
                bitrate      INTEGER,
                samplerate   INTEGER,
                hash         TEXT NOT NULL DEFAULT '',
                cover_path   TEXT,
                favourite    INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS playlists (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id INTEGER NOT NULL,
                track_id    INTEGER NOT NULL,
                position    INTEGER NOT NULL,
                PRIMARY KEY (playlist_id, track_id),
                FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);
            CREATE INDEX IF NOT EXISTS idx_tracks_fav ON tracks(favourite);",
        )
        .map_err(|e| format!("db init: {e}"))?;
        Ok(Self {
            conn,
            _watch_dirs: Mutex::new(Vec::new()),
        })
    }

    pub fn list_tracks(&self) -> Result<Vec<TrackInfo>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite FROM tracks ORDER BY title ASC")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map([], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn get_track(&self, id: i64) -> Result<Option<TrackInfo>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite FROM tracks WHERE id = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        let mut rows = stmt
            .query_map(params![id], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        match rows.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(e)) => Err(format!("row: {e}")),
            None => Ok(None),
        }
    }

    pub fn add_track(&self, path: &str, cache_dir: Option<&str>) -> Result<TrackInfo, String> {
        if self.track_by_path(path)?.is_some() {
            return Err("track already exists".to_string());
        }

        let (meta, hash) = extract_metadata(path, cache_dir)?;

        self.conn
            .execute(
                "INSERT INTO tracks (path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    path,
                    meta.title,
                    meta.artist,
                    meta.album,
                    meta.duration,
                    meta.track_number,
                    meta.genre,
                    meta.year,
                    meta.bitrate,
                    meta.samplerate,
                    hash,
                    meta.cover_path,
                ],
            )
            .map_err(|e| format!("insert: {e}"))?;

        let id = self.conn.last_insert_rowid();
        self.get_track(id)?
            .ok_or_else(|| "inserted track not found".to_string())
    }

    pub fn remove_track(&self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute("DELETE FROM tracks WHERE id = ?1", params![id])
            .map_err(|e| format!("delete: {e}"))?;
        if affected == 0 {
            return Err("track not found".to_string());
        }
        Ok(())
    }

    pub fn update_cover_path(&self, id: i64, cover_path: &str) -> Result<(), String> {
        let affected = self
            .conn
            .execute(
                "UPDATE tracks SET cover_path = ?1 WHERE id = ?2",
                params![cover_path, id],
            )
            .map_err(|e| format!("update cover_path: {e}"))?;
        if affected == 0 {
            return Err("track not found".to_string());
        }
        Ok(())
    }

    pub fn update_metadata(&self, id: i64, patch: &MetadataPatch) -> Result<(), String> {
        let mut sets = Vec::new();
        if patch.title.is_some() {
            sets.push("title = ?2");
        }
        if patch.artist.is_some() {
            sets.push("artist = ?3");
        }
        if patch.album.is_some() {
            sets.push("album = ?4");
        }
        if patch.genre.is_some() {
            sets.push("genre = ?5");
        }
        if patch.year.is_some() {
            sets.push("year = ?6");
        }
        if patch.track_number.is_some() {
            sets.push("track_number = ?7");
        }
        if sets.is_empty() {
            return Ok(());
        }
        let sql = format!("UPDATE tracks SET {} WHERE id = ?1", sets.join(", "));
        self.conn
            .execute(
                &sql,
                params![
                    id,
                    patch.title,
                    patch.artist,
                    patch.album,
                    patch.genre,
                    patch.year,
                    patch.track_number
                ],
            )
            .map_err(|e| format!("update metadata: {e}"))?;
        Ok(())
    }

    pub fn toggle_favourite(&self, id: i64) -> Result<bool, String> {
        self.conn
            .execute(
                "UPDATE tracks SET favourite = CASE WHEN favourite = 0 THEN 1 ELSE 0 END WHERE id = ?1",
                params![id],
            )
            .map_err(|e| format!("toggle fav: {e}"))?;
        let val: i32 = self
            .conn
            .query_row(
                "SELECT favourite FROM tracks WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| format!("read fav: {e}"))?;
        Ok(val != 0)
    }

    pub fn get_favourites(&self) -> Result<Vec<TrackInfo>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite FROM tracks WHERE favourite = 1 ORDER BY title ASC")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map([], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn create_playlist(&self, name: &str) -> Result<Playlist, String> {
        self.conn
            .execute("INSERT INTO playlists (name) VALUES (?1)", params![name])
            .map_err(|e| format!("create playlist: {e}"))?;
        let id = self.conn.last_insert_rowid();
        self.get_playlist(id)?
            .ok_or_else(|| "created playlist not found".to_string())
    }

    pub fn delete_playlist(&self, id: i64) -> Result<(), String> {
        let affected = self
            .conn
            .execute("DELETE FROM playlists WHERE id = ?1", params![id])
            .map_err(|e| format!("delete playlist: {e}"))?;
        if affected == 0 {
            return Err("playlist not found".to_string());
        }
        Ok(())
    }

    pub fn add_to_playlist(&self, playlist_id: i64, track_id: i64) -> Result<(), String> {
        let max_pos: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("max pos: {e}"))?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, track_id, max_pos + 1],
            )
            .map_err(|e| format!("add to playlist: {e}"))?;
        Ok(())
    }

    pub fn remove_from_playlist(&self, playlist_id: i64, track_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
                params![playlist_id, track_id],
            )
            .map_err(|e| format!("remove from playlist: {e}"))?;
        Ok(())
    }

    pub fn get_playlist_tracks(&self, id: i64) -> Result<Vec<TrackInfo>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.id, t.path, t.title, t.artist, t.album, t.duration, t.track_number, t.genre, t.year, t.bitrate, t.samplerate, t.hash, t.cover_path, t.favourite
                 FROM tracks t
                 JOIN playlist_tracks pt ON pt.track_id = t.id
                 WHERE pt.playlist_id = ?1
                 ORDER BY pt.position ASC",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![id], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn get_playlists(&self) -> Result<Vec<Playlist>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, p.created_at,
                        (SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = p.id) AS track_count
                 FROM playlists p ORDER BY p.name ASC",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    track_count: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn get_recent(&self, count: u64) -> Result<Vec<TrackInfo>, String> {
        let limit = if count > 0 { count as i64 } else { 50 };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite
                 FROM tracks ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![limit], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn get_playlist(&self, id: i64) -> Result<Option<Playlist>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, p.created_at,
                        (SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = p.id) AS track_count
                 FROM playlists p WHERE p.id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    track_count: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        match rows.next() {
            Some(Ok(p)) => Ok(Some(p)),
            Some(Err(e)) => Err(format!("row: {e}")),
            None => Ok(None),
        }
    }

    pub fn import_m3u(&self, path: &str) -> Result<Playlist, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("read m3u: {e}"))?;

        let name = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported")
            .to_string();

        let playlist = self.create_playlist(&name)?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let abs_path = if std::path::Path::new(line).is_absolute() {
                line.to_string()
            } else {
                let base = std::path::Path::new(path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                base.join(line).to_string_lossy().to_string()
            };

            match self.add_track(&abs_path, None) {
                Ok(track) => {
                    let _ = self.add_to_playlist(playlist.id, track.id);
                }
                Err(e) => warn!("skipping {abs_path}: {e}"),
            }
        }

        Ok(playlist)
    }

    pub fn export_m3u(&self, playlist_id: i64, path: &str) -> Result<(), String> {
        let playlist = self
            .get_playlist(playlist_id)?
            .ok_or("playlist not found")?;
        let track_ids = self.get_playlist_tracks(playlist_id)?;
        let mut lines = vec![
            "#EXTM3U".to_string(),
            format!("#PLAYLIST: {}", playlist.name),
        ];
        for track in &track_ids {
            let dur_secs = track.duration as u64;
            lines.push(format!(
                "#EXTINF:{},{} - {}",
                dur_secs, track.artist, track.title
            ));
            lines.push(track.path.clone());
        }
        std::fs::write(path, lines.join("\n")).map_err(|e| format!("write m3u: {e}"))?;
        Ok(())
    }

    pub fn scan_directory(
        &self,
        dir: &str,
        recursive: bool,
        cache_dir: Option<&str>,
    ) -> Result<Vec<TrackInfo>, String> {
        let mut added = Vec::new();
        let extensions = ["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus"];

        let walk = if recursive {
            walkdir::WalkDir::new(dir).follow_links(true)
        } else {
            walkdir::WalkDir::new(dir).max_depth(1).follow_links(true)
        };

        for entry in walk.into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !extensions.contains(&ext.to_lowercase().as_str()) {
                continue;
            }
            let path = entry.path().to_string_lossy().to_string();
            match self.add_track(&path, cache_dir) {
                Ok(t) => added.push(t),
                Err(e) => warn!("skip {path}: {e}"),
            }
        }

        Ok(added)
    }

    pub fn search_tracks(&self, query: &str) -> Result<Vec<TrackInfo>, String> {
        let pattern = format!("%{}%", query);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite
                 FROM tracks
                 WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1
                 ORDER BY title ASC
                 LIMIT 10",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![pattern], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("rows: {e}"))
    }

    pub fn track_by_path(&self, path: &str) -> Result<Option<TrackInfo>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title, artist, album, duration, track_number, genre, year, bitrate, samplerate, hash, cover_path, favourite FROM tracks WHERE path = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        let mut rows = stmt
            .query_map(params![path], Self::row_to_track)
            .map_err(|e| format!("query: {e}"))?;
        match rows.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(e)) => Err(format!("row: {e}")),
            None => Ok(None),
        }
    }

    fn row_to_track(row: &rusqlite::Row) -> rusqlite::Result<TrackInfo> {
        Ok(TrackInfo {
            id: row.get(0)?,
            path: row.get(1)?,
            title: row.get(2)?,
            artist: row.get(3)?,
            album: row.get(4)?,
            duration: row.get(5)?,
            track_number: row.get(6)?,
            genre: row.get(7)?,
            year: row.get(8)?,
            bitrate: row.get(9)?,
            samplerate: row.get(10)?,
            hash: row.get(11)?,
            cover_path: row.get(12)?,
            favourite: row.get::<_, i32>(13)? != 0,
            ..Default::default()
        })
    }

    pub fn track_path(&self, id: i64) -> Result<String, String> {
        self.conn
            .query_row(
                "SELECT path FROM tracks WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| format!("track path: {e}"))
    }
}

struct Metadata {
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: Option<i32>,
    track_number: Option<i32>,
    duration: f64,
    bitrate: Option<i32>,
    samplerate: Option<i32>,
    cover_path: Option<String>,
}

fn tag_title(dst: &mut String, tag: &symphonia::core::meta::Tag) {
    if !dst.is_empty() {
        return;
    }
    *dst = tag.raw.value.to_string();
}

fn extract_metadata(path: &str, cache_dir: Option<&str>) -> Result<(Metadata, String), String> {
    let cache_base = cache_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("gtm")
        })
        .join("covers");
    fs::create_dir_all(&cache_base).ok();

    let hash = {
        let mut hasher = Sha256::new();
        let mut f = File::open(path).map_err(|e| format!("open hash: {e}"))?;
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            let n = f.read(&mut buf).map_err(|e| format!("read hash: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        hex::encode(hasher.finalize())
    };

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return Err(format!("open: {e}")),
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let fmt_opts = FormatOptions::default();
    let meta_opts = MetadataOptions::default();

    let probe = symphonia::default::get_probe();
    let mut reader = match probe.probe(&hint, mss, fmt_opts, meta_opts) {
        Ok(r) => r,
        Err(e) => return Err(format!("probe: {e}")),
    };

    let mut title = String::new();
    let mut artist = String::new();
    let mut album = String::new();
    let mut genre = String::new();
    let mut year: Option<i32> = None;
    let mut track_number: Option<i32> = None;
    let bitrate: Option<i32> = None;
    let samplerate: Option<i32> = None;

    {
        let meta = reader.metadata();
        if let Some(rev) = meta.current() {
            for tag in &rev.media.tags {
                match &tag.std {
                    Some(StandardTag::TrackTitle(_)) => tag_title(&mut title, tag),
                    Some(StandardTag::Artist(_)) => tag_title(&mut artist, tag),
                    Some(StandardTag::Album(_)) => tag_title(&mut album, tag),
                    Some(StandardTag::Genre(_)) => tag_title(&mut genre, tag),
                    Some(StandardTag::TrackNumber(n)) => track_number = Some(*n as i32),
                    Some(StandardTag::RecordingYear(n))
                    | Some(StandardTag::ReleaseYear(n))
                    | Some(StandardTag::OriginalReleaseYear(n))
                    | Some(StandardTag::OriginalRecordingYear(n))
                        if year.is_none() =>
                    {
                        year = Some(*n as i32);
                    }
                    _ => {}
                }
            }
        }
    }

    let mut cover_path: Option<String> = None;
    {
        let meta = reader.metadata();
        if let Some(rev) = meta.current() {
            for visual in &rev.media.visuals {
                let is_cover = matches!(visual.usage, Some(StandardVisualKey::FrontCover));
                if is_cover || visual.usage.is_none() {
                    let ext = match visual.media_type.as_deref() {
                        Some("image/jpeg" | "image/jpg") => "jpg",
                        Some("image/png") => "png",
                        _ => "jpg",
                    };
                    let cover_file = cache_base.join(format!("{}.{}", hash, ext));
                    if !cover_file.exists() {
                        if let Ok(mut buf) = File::create(&cover_file) {
                            use std::io::Write;
                            let _ = buf.write_all(&visual.data);
                        }
                    }
                    cover_path = Some(cover_file.to_string_lossy().to_string());
                    break;
                }
            }
        }
    }

    let duration = reader
        .tracks()
        .iter()
        .filter_map(|t| {
            let tb = t.time_base.as_ref()?;
            let dur = t.duration?;
            let ts = Timestamp::new(dur.get() as i64);
            let time = tb.calc_time(ts)?;
            Some(time.as_secs_f64())
        })
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0);

    if artist.is_empty() || title.is_empty() {
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        // Use metadata_cleaner for comprehensive YouTube title cleaning
        let (cleaned_artist, cleaned_title) = crate::metadata_cleaner::clean_youtube_title(stem);
        if title.is_empty() && !cleaned_title.is_empty() {
            title = cleaned_title;
        }
        if artist.is_empty() {
            if let Some(a) = cleaned_artist {
                artist = a;
            } else if let Some(dash_idx) = stem.find(" - ") {
                // Fallback: split on first " - "
                if dash_idx > 0 {
                    artist = stem[..dash_idx].trim().to_string();
                }
                let after_dash = stem[dash_idx + 3..].trim();
                let mut clean = after_dash.to_string();
                loop {
                    let prev = clean.clone();
                    let trimmed = prev.trim_end();
                    let next = if trimmed.ends_with(')') {
                        trimmed
                            .rfind('(')
                            .filter(|&o| o > 0)
                            .map(|o| trimmed[..o].trim_end().to_string())
                    } else if trimmed.ends_with(']') {
                        trimmed
                            .rfind('[')
                            .filter(|&o| o > 0)
                            .map(|o| trimmed[..o].trim_end().to_string())
                    } else {
                        None
                    };
                    match next {
                        Some(s) if !s.is_empty() && s != clean => clean = s,
                        _ => break,
                    }
                }
                if !clean.is_empty() && title.is_empty() {
                    title = clean;
                }
            }
        }
        if title.is_empty() {
            title = stem.to_string();
        }
    }

    Ok((
        Metadata {
            title,
            artist,
            album,
            genre,
            year,
            track_number,
            duration,
            bitrate,
            samplerate,
            cover_path,
        },
        hash,
    ))
}
