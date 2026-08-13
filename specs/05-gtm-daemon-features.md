# 05 — gtm-daemon: Features (yt-dlp, Cover Art, Lyrics, Queue, Crossfade)

## Purpose

Additional subsystems that live inside the daemon process: YouTube/yt-dlp streaming manager,
Deezer cover art cache, LRC lyrics resolution, queue manager, and crossfade logic.

## yt-dlp Manager

### Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtSearchResult {
    pub id: String,                     // YouTube video ID (11 chars)
    pub title: String,
    pub url: String,                    // "https://www.youtube.com/watch?v=<id>"
    pub channel: String,
    pub duration: f64,                  // seconds
    pub views: u64,
    pub thumbnail: Option<String>,      // "https://i.ytimg.com/vi/<id>/hqdefault.jpg"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,                    // direct media URL (expires)
    pub title: String,
    pub ext: String,                    // "webm", "m4a"
    pub duration: f64,
}
```

### YoutubeManager struct

```rust
pub struct YoutubeManager {
    subprocess: Option<tokio::process::Child>,
    results: Vec<YtSearchResult>,
    search_query: Option<String>,
    search_filter: Option<YtFilter>,
    cancel_token: CancellationToken,
    concurrent_jobs: Semaphore,        // max 2
}

impl YoutubeManager {
    pub fn new() -> Self;

    // ─── Search ───

    /// Spawn yt-dlp to search. Non-blocking.
    /// yt-dlp ytsearch{count}:{query} --dump-json --flat-playlist
    /// where count = 10 by default.
    pub async fn search(&mut self, query: &str, filter: Option<YtFilter>) -> Result<()>;

    /// Poll for completed results. Returns None if search still running.
    pub async fn poll_results(&mut self) -> Result<Option<Vec<YtSearchResult>>>;

    /// Cancel running search (kills subprocess).
    pub fn cancel(&mut self);

    // ─── Stream resolution ───

    /// Resolve a YouTube URL to a direct stream URL.
    /// Spawns: yt-dlp -g -f bestaudio <url>
    pub async fn resolve_stream(&mut self, url: &str) -> Result<StreamInfo>;

    // ─── Direct download ───

    /// Download a YouTube video as audio file.
    /// Returns the output file path.
    pub async fn download(&mut self, url: &str, output_dir: &Path) -> Result<String>;
}
```

### Subprocess management

```
yt-dlp invocation for search:
  yt-dlp "ytsearch10:{query}" --dump-json --flat-playlist --no-warnings
  → stdout: JSON lines (one per result)
  → stderr: logged at debug, checked for "ERROR" keywords

yt-dlp invocation for resolve:
  yt-dlp -g -f "bestaudio[ext=m4a]/bestaudio" "{url}"
  → stdout: single line with direct URL
  → stderr: ignored (progress info)

yt-dlp invocation for download:
  yt-dlp -x --audio-format mp3 --output "{output_dir}/%(title)s.%(ext)s" "{url}"
  → stdout: progress lines
  → stderr: warnings

Constraints:
  • Max 2 concurrent yt-dlp subprocesses (Semaphore with 2 permits)
  • Timeout: 30s per operation
  • Cancel: kills subprocess via Child::kill()
```

## Cover Art Cache

### Types

```rust
/// Cover image data sent to TUI via MetadataChanged or Custom DaemonEvent.
/// data: raw image bytes (JPEG or PNG)
/// mime: "image/jpeg" or "image/png"
pub struct CoverData {
    pub data: Vec<u8>,
    pub mime: String,
}
```

### CoverCache struct

```rust
pub struct CoverCache {
    memory_cache: LruCache<String, CoverData>,  // max 500 entries
    cache_dir: PathBuf,
    http_client: reqwest::Client,
}

impl CoverCache {
    pub fn new(cache_dir: PathBuf) -> Self;

    /// Look up cover by artist + album. Checks:
    ///   1. Memory cache (LRU, 500 entries)
    ///   2. Disk cache ($cache_dir/covers/{hash}.jpg)
    ///   3. Deezer API (search → download → save)
    pub async fn get_cover(&mut self, artist: &str, album: &str) -> Option<CoverData>;

    /// Look up cover from embedded file metadata.
    /// Extracted during metadata scan, saved to disk cache.
    pub async fn cache_embedded(&mut self, artist: &str, album: &str, data: &[u8]) -> Result<()>;

    /// Generate cache key from artist + album.
    fn cache_key(artist: &str, album: &str) -> String;  // SHA256(artist:album)[:16]
}
```

### Deezer API Integration

```
Deezer Search:
  GET https://api.deezer.com/search?q=artist:"<artist>" album:"<album>"
  Headers: None needed (public API)

Response shape (JSON):
  {
    "data": [
      {
        "id": 12345,
        "title": "<album name>",
        "artist": { "name": "<artist name>" },
        "album": { "title": "<album name>" },
        "cover_medium": "https://e-cdns-images.dzcdn.net/images/cover/<hash>/250x250.jpg",
        "cover_big": "https://e-cdns-images.dzcdn.net/images/cover/<hash>/500x500.jpg"
      }
    ]
  }

Strategy:
  1. URL-encode artist and album
  2. Parse JSON response, pick first result
  3. Try cover_big first, fall back to cover_medium
  4. Download image bytes via reqwest
  5. Save to disk as JPEG: {cache_dir}/covers/{hash}.jpg
  6. Return CoverData { data: bytes, mime: "image/jpeg" }

Rate limiting:
  • Max 5 requests per second (enforced by sleep between calls)
  • No API key needed
```

## Lyrics Manager

### LyricsManager struct

```rust
pub struct LyricsManager {
    http_client: reqwest::Client,
    cache: Library,     // shares the SQLite connection for lyrics_cache table
}

impl LyricsManager {
    pub fn new(cache: Library) -> Self;

    /// Get lyrics for a track. Resolution order:
    ///   1. Sidecar .lrc file next to track
    ///   2. LRCLIB exact match by artist+title+album
    ///   3. LRCLIB fuzzy search by query
    ///   4. SQLite lyrics_cache table
    /// Results stored in lyrics_cache on successful fetch.
    pub async fn get_lyrics(&self, track: &TrackInfo) -> Option<LrcData>;

    /// Parse an LRC file content.
    pub fn parse_lrc(content: &str) -> LrcData;

    /// Compute cache key: SHA256(artist:album:title)[:16]
    pub fn cache_key(track: &TrackInfo) -> String;
}
```

### LRC Parser

```
[ti:Song Title]
[ar:Artist Name]
[al:Album Name]
[00:12.34]First line of lyrics
[00:15.67]Second line
[00:18.90]Third line

→ LrcData {
    title: Some("Song Title"),
    artist: Some("Artist Name"),
    album: Some("Album Name"),
    lines: vec![
        LrcLine { timestamp: 12.34, text: "First line of lyrics" },
        LrcLine { timestamp: 15.67, text: "Second line" },
        LrcLine { timestamp: 18.90, text: "Third line" },
    ]
}

Current line lookup: binary search for largest timestamp ≤ current_position

Time format variants:
  [mm:ss.xx]     → seconds = mm * 60 + ss.xx
  [mm:ss.xxx]    → seconds = mm * 60 + ss.xxx
  [hh:mm:ss.xx]  → seconds = hh * 3600 + mm * 60 + ss.xx
```

### LRCLIB API Integration

```
Exact match:
  GET https://lrclib.net/api/get?artist=<artist>&title=<title>&album=<album>
  Response 200: { "syncLyrics": "[00:12.34]...", "plainLyrics": "...", ... }
  Response 404: not found

Fuzzy search:
  GET https://lrclib.net/api/search?q=<artist> <title>
  Response 200: [{ "id": ..., "artistName": "...", "trackName": "...", ... }]
  Pick best match (exact artist+title match preferred, else first result)

Response shape:
  {
    "id": 12345,
    "name": "track name",
    "artistName": "artist name",
    "albumName": "album name",
    "duration": 260.0,
    "syncLyrics": "[00:12.34]...",
    "plainLyrics": "First line...",
    "isSynced": true,
    "updatedAt": "2024-01-15T12:00:00Z"
  }

Strategy:
  1. Try exact match first
  2. If 404, try fuzzy search
  3. Prefer synced lyrics; fall back to plain lyrics
  4. Cache in SQLite lyrics_cache table
```

## Queue Manager

### QueueManager struct

```rust
pub struct QueueManager {
    pub queue: Vec<TrackInfo>,
    pub cursor: usize,
    pub shuffle_order: Vec<usize>,
    pub shuffle_cursor: usize,
    pub repeat: RepeatMode,
    pub shuffle: bool,
}

impl QueueManager {
    pub fn new() -> Self;

    /// Current track, considering shuffle mode.
    pub fn current(&self) -> Option<&TrackInfo>;

    /// Advance to next track. Returns new cursor position.
    /// Returns None if at end and repeat=Off (playback stops).
    pub fn advance(&mut self) -> Option<usize>;

    /// Go to previous track.
    pub fn prev(&mut self) -> usize;

    /// Fisher-Yates shuffle, preserving current track at front.
    pub fn reshuffle(&mut self);

    /// Set entire queue, starting at given cursor.
    pub fn set_queue(&mut self, tracks: Vec<TrackInfo>, start: usize);

    /// Add track at position (None = append).
    pub fn add(&mut self, track: TrackInfo, position: Option<usize>);

    /// Remove track at index. Adjusts cursor if needed.
    pub fn remove(&mut self, index: usize);

    /// Move track from one position to another.
    pub fn move_item(&mut self, from: usize, to: usize);

    /// Clear the queue.
    pub fn clear(&mut self);
}
```

### Queue Logic

```
Normal mode:
  advance():
    if repeat == One:       return cursor (same track)
    if repeat == All:       cursor = (cursor + 1) % len
    if repeat == Off:
      if cursor + 1 < len:  cursor += 1
      else:                  return None (stop)

Shuffle mode:
  advance():
    if repeat == One:       return shuffle_order[shuffle_cursor] (same)
    shuffle_cursor += 1
    if shuffle_cursor >= shuffle_order.len(): reshuffle()
    return Some(shuffle_order[shuffle_cursor])

  reshuffle():
    1. Preserve current track: swap shuffle_order[0] with shuffle_order[sc]
    2. Fisher-Yates on shuffle_order[1..]
    3. shuffle_cursor = 0

  set_queue():
    shuffle_order = (0..len).collect()
    if shuffle: reshuffle()
    cursor = start.clamp(0, len - 1)

  remove(index):
    Remove from queue.
    If remove before cursor: cursor -= 1
    If remove is cursor and queue not empty: cursor = min(cursor, len - 1)
    Rebuild shuffle_order if shuffle.

  prev():
    if shuffle && shuffle_cursor > 0: shuffle_cursor -= 1
    else if cursor > 0: cursor -= 1
    else: cursor = 0
```

## Crossfade

```rust
pub struct CrossfadeConfig {
    pub enabled: bool,
    pub duration_secs: u8,    // default 3, range 1-30
}
```

### Crossfade Flow

```
Crossfade is handled entirely by the daemon, not the audio backend.

when current_track reaches (duration - crossfade_duration):
    → start_crossfade(next_track)

async fn start_crossfade(d: &mut Daemon, next: &TrackInfo) {
    // 1. Determine if fade-out is needed (track playing, not paused)
    let fade_out = d.state.status == PlaybackStatus::Playing;

    // 2. Pre-spawn the next track's backend instance
    //    (only if using dual-backend approach)
    let fade_duration = d.state.crossfade.as_ref().unwrap().duration_secs as f64;

    if fade_out {
        // Ramp current volume → 0 over fade_duration seconds
        let start_vol = d.state.volume;
        let steps = (fade_duration * 10.0) as u32;  // 100ms per step
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let vol = (start_vol as f64 * (1.0 - t)).round() as u8;
            d.backend.set_volume(vol).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // 3. Load and play next track
    d.backend.load(&next.path, 0.0).await.unwrap();
    d.backend.set_volume(d.state.volume).await;
    d.backend.play().await.unwrap();

    // 4. Update state
    d.state.current_track = Some(next.clone());
    d.state.time_pos = 0.0;
    d.push_event(DaemonEvent::PlaybackStarted {
        track: next.clone(),
        auto_advanced: true,
        time_pos: 0.0,
        duration: next.duration,
    });
}

Simpler single-backend approach (recommended for v1):
  1. Backend reaches end of track → Finished event
  2. Daemon handles Finished by advancing queue
  3. New track loaded with backend.load() — gap is minimal (~50ms)
  4. Fade effect can be approximated by starting next track at volume 0
     and ramping up over crossfade_duration
```

## File Structure

```
gtmd/src/
├── youtube.rs         # YoutubeManager
├── cover_art.rs       # CoverCache
├── lyrics.rs          # LyricsManager
└── queue.rs           # QueueManager
```
