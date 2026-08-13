//! Queue management helpers: add, remove, move, clear, scan.
//!
//! Tracks are stored as `Vec<TrackInfo>` in `DaemonState.queue` with a
//! `queue_cursor` pointing to the currently-playing entry.  Metadata
//! resolution is minimal (file stem → title); full metadata extraction
//! is future work.

use std::path::Path;

use gtm_core::state::DaemonState;
use gtm_core::track::TrackInfo;

/// Build a bare TrackInfo from a file path.  The title is derived from
/// the file stem; all other fields are left empty/default.
pub fn resolve_track(path: &str) -> TrackInfo {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    TrackInfo {
        id: 0,
        path: path.to_string(),
        title: stem,
        artist: String::new(),
        album: String::new(),
        duration: 0.0,
        track_number: None,
        genre: String::new(),
        year: None,
        bitrate: None,
        samplerate: None,
        hash: String::new(),
        cover_path: None,
        favourite: false,
    }
}

pub fn queue_add(state: &mut DaemonState, path: &str, position: Option<u128>) -> TrackInfo {
    let track = resolve_track(path);
    let track_clone = track.clone();
    match position {
        Some(pos) if pos < state.queue.len() as u128 => {
            state.queue.insert(pos as usize, track);
        }
        _ => {
            state.queue.push(track);
        }
    }
    track_clone
}

pub fn queue_add_many(state: &mut DaemonState, paths: &[String]) -> Vec<TrackInfo> {
    let mut added = Vec::with_capacity(paths.len());
    for path in paths {
        let track = resolve_track(path);
        let track_clone = track.clone();
        state.queue.push(track);
        added.push(track_clone);
    }
    added
}

pub fn queue_remove(state: &mut DaemonState, index: u128) -> Option<TrackInfo> {
    if index >= state.queue.len() as u128 {
        return None;
    }
    let removed = state.queue.remove(index as usize);
    if state.queue_cursor > index && state.queue_cursor > 0 {
        state.queue_cursor -= 1;
    }
    state.queue_cursor = state
        .queue_cursor
        .min(state.queue.len().saturating_sub(1) as u128);
    Some(removed)
}

pub fn queue_move(state: &mut DaemonState, from: u128, to: u128) -> bool {
    let len = state.queue.len() as u128;
    if from >= len || to >= len || from == to {
        return false;
    }
    let track = state.queue.remove(from as usize);
    state.queue.insert(to as usize, track);
    // Adjust cursor if needed
    if state.queue_cursor == from {
        state.queue_cursor = to;
    } else if from < state.queue_cursor && to >= state.queue_cursor {
        state.queue_cursor -= 1;
    } else if from > state.queue_cursor && to <= state.queue_cursor {
        state.queue_cursor += 1;
    }
    true
}

pub fn queue_set(state: &mut DaemonState, paths: &[String], start_idx: u128) -> Vec<TrackInfo> {
    let mut tracks = Vec::with_capacity(paths.len());
    for path in paths {
        tracks.push(resolve_track(path));
    }
    state.queue = tracks;
    state.queue_cursor = start_idx.min(state.queue.len().saturating_sub(1) as u128);
    state.queue.clone()
}

pub fn queue_clear(state: &mut DaemonState) {
    state.queue.clear();
    state.queue_cursor = 0;
}

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus", "wma"];

pub fn scan_audio_files(path: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        if dir.is_file() {
            if let Some(ext) = dir
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
            {
                if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                    paths.push(path.to_string());
                }
            }
        }
        return paths;
    }
    for entry in walkdir::WalkDir::new(dir).follow_links(true) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if let Some(ext) = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        {
            if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                paths.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    paths
}
