# 04 — gtm-daemon: Library (SQLite)

## Purpose

Manage the local music library database: track metadata, playlists, favourites, search history,
lyrics cache, download queue. All SQLite access goes through the `Library` struct.

Depends on: `rusqlite` (bundled sqlite3 via feature), `gtm-core`

## Schema

```sql
# Core track table
CREATE TABLE IF NOT EXISTS tracks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT UNIQUE NOT NULL,
    title       TEXT NOT NULL DEFAULT '',
    artist      TEXT NOT NULL DEFAULT '',
    album       TEXT NOT NULL DEFAULT '',
    duration    REAL NOT NULL DEFAULT 0.0,
    track_no    INTEGER,
    genre       TEXT NOT NULL DEFAULT '',
    year        INTEGER,
    bitrate     INTEGER,
    samplerate  INTEGER,
    hash        TEXT NOT NULL DEFAULT '',
    cover_path  TEXT,
    added_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);
CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS idx_tracks_album ON tracks(album);
CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);

-- Playlists
CREATE TABLE IF NOT EXISTS playlists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Playlist-tracks junction
CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    added_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (playlist_id, position)
);
CREATE INDEX IF NOT EXISTS idx_pl_tracks_playlist ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_pl_tracks_track ON playlist_tracks(track_id);

-- Favourites
CREATE TABLE IF NOT EXISTS favourites (
    track_id  INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
    added_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Move-to-trash (soft delete)
CREATE TABLE IF NOT EXISTS trash (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    deleted_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Lyrics cache (key = SHA256(artist+album+title)[:16])
CREATE TABLE IF NOT EXISTS lyrics_cache (
    track_key   TEXT PRIMARY KEY,
    source      TEXT NOT NULL DEFAULT 'local',
    lyrics_data TEXT NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Search history
CREATE TABLE IF NOT EXISTS search_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    query       TEXT NOT NULL,
    searched_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_search_history_time ON search_history(searched_at DESC);

-- Downloads (yt-dlp queue)
CREATE TABLE IF NOT EXISTS downloads (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    url         TEXT NOT NULL,
    title       TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    progress    REAL NOT NULL DEFAULT 0.0,
    path        TEXT,
    added_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
```

## Library Struct

```rust
pub struct Library {
    conn: Connection,
    watch_paths: Vec<PathBuf>,
    scan_batch_size: usize,
}

impl Library {
    /// Open or create database at path. Runs all CREATE TABLE IF NOT EXISTS.
    pub fn open(path: &Path) -> Result<Self>;

    // ─── Track CRUD ───

    /// Add a single track file. Returns track id.
    /// Extracts metadata via MetadataExtractor.
    /// Skips duplicates (matched by path) — updates hash if changed.
    pub fn add_track(&mut self, path: &Path) -> Result<i64>;

    /// Scan a directory recursively. Spawn_blocking, reports via watch channel.
    pub fn scan_directory(&mut self, path: &Path) -> Result<watch::Receiver<ScanProgress>>;

    /// Remove a track by id (hard delete).
    pub fn remove_track(&mut self, id: i64) -> Result<()>;

    // ─── Queries ───

    /// Get tracks with optional filter + sort.
    /// Filter: SQL WHERE on title/artist/album using LIKE.
    /// Sort: "artist", "album", "title", "duration", "year", "added_at".
    pub fn get_tracks(&self, filter: Option<&str>, sort: Option<&str>) -> Result<Vec<TrackInfo>>;

    /// Full-text search across title, artist, album using LIKE.
    pub fn search_tracks(&self, query: &str) -> Result<Vec<TrackInfo>>;

    pub fn get_track_by_id(&self, id: i64) -> Result<Option<TrackInfo>>;
    pub fn get_track_by_path(&self, path: &str) -> Result<Option<TrackInfo>>;

    /// Recently added tracks, ordered by added_at DESC.
    pub fn get_recent_tracks(&self, count: usize) -> Result<Vec<TrackInfo>>;

    /// All tracks grouped by album/artist for browsing.
    pub fn get_albums(&self) -> Result<Vec<String>>;
    pub fn get_artists(&self) -> Result<Vec<String>>;

    // ─── Playlists ───

    pub fn create_playlist(&mut self, name: &str) -> Result<i64>;
    pub fn delete_playlist(&mut self, id: i64) -> Result<()>;
    pub fn get_playlists(&self) -> Result<Vec<Playlist>>;
    pub fn get_playlist_tracks(&self, id: i64) -> Result<Vec<TrackInfo>>;
    pub fn add_to_playlist(&mut self, playlist_id: i64, track_ids: &[i64]) -> Result<()>;
    pub fn remove_from_playlist(&mut self, playlist_id: i64, position: usize) -> Result<()>;

    // ─── Playlist I/O ───

    /// Import an M3U file. Returns number of tracks imported.
    /// M3U format: #EXTINF:<duration>,<artist> - <title> then next line is path.
    /// Also parses plain M3U (paths only, no #EXTINF).
    pub fn import_m3u(&mut self, path: &Path) -> Result<usize>;

    /// Export a playlist to M3U file.
    pub fn export_m3u(&self, playlist_id: i64, path: &Path) -> Result<()>;

    // ─── Favourites ───

    pub fn add_favourite(&mut self, track_id: i64) -> Result<()>;
    pub fn remove_favourite(&mut self, track_id: i64) -> Result<()>;
    pub fn get_favourites(&self) -> Result<Vec<TrackInfo>>;
    pub fn is_favourite(&self, track_id: i64) -> bool;
}
```

## Full SQL Query Reference

### get_tracks

```sql
SELECT id, path, title, artist, album, duration, track_no, genre,
       year, bitrate, samplerate, hash, cover_path,
       (SELECT 1 FROM favourites WHERE track_id = tracks.id) AS favourite
FROM tracks
WHERE (title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1)
ORDER BY {sort_column} {sort_dir}
```

### search_tracks

```sql
SELECT id, path, title, artist, album, duration, track_no, genre,
       year, bitrate, samplerate, hash, cover_path,
       (SELECT 1 FROM favourites WHERE track_id = tracks.id) AS favourite
FROM tracks
WHERE title LIKE '%' || ?1 || '%'
   OR artist LIKE '%' || ?1 || '%'
   OR album LIKE '%' || ?1 || '%'
ORDER BY title ASC
```

### get_playlist_tracks

```sql
SELECT t.id, t.path, t.title, t.artist, t.album, t.duration,
       t.track_no, t.genre, t.year, t.bitrate, t.samplerate,
       t.hash, t.cover_path,
       (SELECT 1 FROM favourites WHERE track_id = t.id) AS favourite
FROM tracks t
JOIN playlist_tracks pt ON pt.track_id = t.id
WHERE pt.playlist_id = ?1
ORDER BY pt.position ASC
```

### add_to_playlist

```sql
-- Insert tracks at the end:
INSERT INTO playlist_tracks (playlist_id, track_id, position)
SELECT ?1, value, (SELECT COALESCE(MAX(position), 0) + 1
                    FROM playlist_tracks WHERE playlist_id = ?1)
FROM json_each(?2)  -- track_ids as JSON array
```

### import_m3u

```rust
/// M3U parser logic:
/// Extended M3U (#EXTM3U):
///   #EXTINF:123,Artist Name - Song Title
///   /path/to/file.mp3
///
/// Plain M3U (one path per line, # starts comment):
///   # comment
///   /path/to/file.flac
///   /path/to/another.mp3
///
/// For each path:
///   1. If relative, resolve against M3U file directory
///   2. Call add_track(path) to import (duplicates skipped by path)
///   3. If EXTINF had playlist position info, create playlist
```

## MetadataExtractor

```rust
/// Extracts audio metadata from files using symphonia (primary) or ffmpeg (fallback).

pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Extract tags from a file. Returns (title, artist, album, track_no, genre,
    /// year, bitrate, samplerate, duration, cover_path).
    pub fn extract(path: &Path) -> Result<ExtractedMetadata>;

    /// Compute SHA-256 of first 64KB of file content.
    pub fn compute_hash(path: &Path) -> Result<String>;

    /// Probe file to determine if it's playable audio.
    pub fn is_playable(path: &Path) -> bool;
}

pub struct ExtractedMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub track_no: Option<i32>,
    pub genre: String,
    pub year: Option<i32>,
    pub bitrate: Option<i32>,
    pub samplerate: Option<i32>,
    pub cover_data: Option<Vec<u8>>,  // raw image bytes for cache
}
```

### Extraction Flow

```
File path
    │
    ▼
┌──────────────────────────┐
│ symphonia::probe()       │ primary, fast, pure Rust
└────────┬─────────────────┘
         │ success?
         ├── YES ──▶ extract tags from format::Metadata → ExtractedMetadata
         │
         └── NO ───▶
                    ┌──────────────────────────┐
                    │ ffmpeg -i <file> -f null  │ fallback (feature-gated)
                    │ parse stderr for metadata  │
                    └────────┬─────────────────┘
                             │
                             ▼
                        ExtractedMetadata (with duration via ffprobe analysis)
```

## Scan Progress

```rust
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub total: usize,
    pub scanned: usize,
    pub added: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub done: bool,
}

/// Scan runs on tokio::task::spawn_blocking.
/// Progress is polled via tokio::sync::watch::Receiver<ScanProgress>.
///
/// Implementation:
///   1. Walk directory recursively for audio files (*.mp3, *.flac, *.ogg,
///      *.wav, *.m4a, *.aac, *.opus, *.wma)
///   2. For each file, call add_track()
///   3. Every batch_size files (default 50), update ScanProgress and
///      send to watch channel
///   4. Return the watch::Receiver immediately; caller polls for progress
```

## Config Paths

```
Database:  $XDG_DATA_HOME/gtm/library.db  (~/.local/share/gtm/library.db)
```

## File Structure

```
gtmd/src/library.rs    # Library struct + all queries
```
