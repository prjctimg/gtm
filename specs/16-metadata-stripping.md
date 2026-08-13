# Spec 16 — Improved Metadata Stripping

## Requirement

> Study the various patterns used for filenames on Youtube and create regular expressions for matching track titles and artist tags and removing filler strings like 'Official Audio' etc.

## Current State

- `gtmd/src/youtube.rs:199`: Title extracted raw from yt-dlp JSON, no cleaning
- `gtm/src/app.rs:1021`: Download output template uses raw `%(title)s`
- `gtmd/src/library.rs:604-619`: Only strips trailing `()` and `[]` brackets when tags are missing — limited to that pattern

## Changes

### 16a. New file: `gtmd/src/metadata_cleaner.rs`

Create `clean_youtube_title(title: &str) -> (Option<String>, String)` returning `(artist_option, cleaned_title)`.

**Regex patterns to strip** (applied in order):
1. Official media tags: `\(\s*Official\s+(Audio|Video|Music\s*Video|Lyric\s*Video|Visualizer)\s*\)` and `[` bracket variants
2. Quality tags: `\[?\b(HD|4K|8K|1080p|720p|480p)\b\]?`
3. Explicit/Clean: `\(\s*(Explicit|Clean)\s*\)`
4. Feature tags: `\s*[|,]?\s*(feat\.?|ft\.?|x)\s+.*` (to end of string)
5. Year: `\(\s*\d{4}\s*\)` and `\[?\s*\d{4}\s*\]?`
6. Generic fillers: `Audio\s+Only`, `With\s+Lyrics`, `Official`, `Music`, `Lyric\s*Video`
7. Topic channel prefix: `^[\w\s]+ - Topic\s*[-–—]\s*`
8. Trailing noise: `[-–—]\s*Topic$`

**Artist extraction**:
- If cleaned title contains ` - ` (space-dash-space): split on first occurrence → `(Some(artist), title)`
- If not: `(None, title)`

**Post-processing**:
- Trim whitespace
- Collapse multiple spaces
- Strip leading/trailing `-`, `|`, `,`

### 16b. `gtmd/src/youtube.rs`
- Import `metadata_cleaner::clean_youtube_title`
- After extracting title at line 199, call `clean_youtube_title()`
- Use the returned artist and title in the search result struct

### 16c. `gtm/src/app.rs`
- In the download handler (line 1021), apply `clean_youtube_title()` to sanitize the output filename
- Replace special filesystem characters (`/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`) with `_`

### 16d. `gtmd/src/lib.rs`
- Add `pub mod metadata_cleaner;`

## Verification
- YouTube search results show cleaned titles (no "Official Audio", "4K", etc.)
- Download filenames are sanitized (no special chars)
- Artist/title split works for "Artist - Title (Official Audio)" patterns
- Edge cases: titles with no filler pass through unchanged
