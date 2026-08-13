// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Queue management helpers: add, remove, move, clear, scan
//
// This is free software released under the GPL-3.0 license.

//! Queue management helpers: add, remove, move, clear, scan.
//!
//! The queue is a one-time FIFO of user-added tracks: the currently-playing
//! entry sits at index 0 and is removed once it finishes or Next is pressed.
//! When the user queue is empty, playback falls back to `default_list` (the
//! whole library sorted by title, or shuffled), whose entries persist while
//! `default_cursor` advances.  Clients see a merged view of both.

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
        ..Default::default()
    }
}

/// The merged queue view shown to clients: user entries followed by the
/// remaining default list.  The cursor marks the currently-playing entry:
/// index 0 when a user entry is playing, otherwise the position in the
/// default list.
pub fn visible_queue(state: &DaemonState) -> (Vec<TrackInfo>, u64) {
    let mut merged = state.queue.clone();
    let cursor = if state.queue.is_empty() {
        state.default_cursor.min(state.default_list.len()) as u64
    } else {
        0
    };
    merged.extend(state.default_list.iter().cloned());
    (merged, cursor)
}

/// Map a merged-view index to its owning structure and local index.
fn split_index(state: &DaemonState, idx: usize) -> Option<(bool, usize)> {
    if idx < state.queue.len() {
        Some((true, idx))
    } else {
        let local = idx - state.queue.len();
        if local < state.default_list.len() {
            Some((false, local))
        } else {
            None
        }
    }
}

/// Insert a track into the merged view at `pos`, maintaining the cursor.
fn insert_at(state: &mut DaemonState, track: TrackInfo, pos: usize) {
    let ulen = state.queue.len();
    if pos <= ulen {
        state.queue.insert(pos, track);
    } else {
        let local = pos - ulen;
        state.default_list.insert(local, track);
        if local <= state.default_cursor {
            state.default_cursor += 1;
        }
    }
}

/// Add a track.  `position == None` queues it to play next (right after the
/// current entry); `Some(pos)` inserts at an explicit merged-view index.
/// Returns the created TrackInfo.
pub fn queue_add(state: &mut DaemonState, path: &str, position: Option<u64>) -> TrackInfo {
    let mut added = queue_add_many(state, &[path.to_string()], position);
    added
        .pop()
        .expect("queue_add_many returns one entry per path")
}

/// Add multiple tracks as a batch.  The whole batch is queued to play next
/// (after the current entry) unless `position` is given, preserving order.
pub fn queue_add_many(
    state: &mut DaemonState,
    paths: &[String],
    position: Option<u64>,
) -> Vec<TrackInfo> {
    let mut added = Vec::with_capacity(paths.len());
    let len = state.queue.len() + state.default_list.len();
    let insert_pos = match position {
        Some(p) => (p as usize).min(len),
        None => {
            if state.queue.is_empty() {
                0
            } else {
                1
            }
        }
    };
    for (i, path) in paths.iter().enumerate() {
        let track = resolve_track(path);
        let track_clone = track.clone();
        insert_at(state, track, insert_pos + i);
        added.push(track_clone);
    }
    added
}

/// Remove the entry at a merged-view index.  Returns the removed track, or
/// None if the index is out of range.
pub fn queue_remove(state: &mut DaemonState, index: u64) -> Option<TrackInfo> {
    let (is_user, local) = split_index(state, index as usize)?;
    let removed = if is_user {
        state.queue.remove(local)
    } else {
        let t = state.default_list[local].clone();
        state.default_list.remove(local);
        if local < state.default_cursor && state.default_cursor > 0 {
            state.default_cursor -= 1;
        }
        t
    };
    Some(removed)
}

/// Move an entry between merged-view indices.  Returns false if either index
/// is out of range.
pub fn queue_move(state: &mut DaemonState, from: u64, to: u64) -> bool {
    let len = state.queue.len() + state.default_list.len();
    let (from, to) = (from as usize, to as usize);
    if from >= len || to >= len || from == to {
        return false;
    }
    let (fuser, flocal) = split_index(state, from).expect("from validated above");
    let track = if fuser {
        state.queue.remove(flocal)
    } else {
        let t = state.default_list.remove(flocal);
        if flocal < state.default_cursor && state.default_cursor > 0 {
            state.default_cursor -= 1;
        }
        t
    };
    let ulen = state.queue.len();
    let tlocal = if to < ulen { to } else { to - ulen };
    insert_at(state, track, tlocal);
    true
}

/// Replace the user queue with `paths` and drop the default-list session.
pub fn queue_set(state: &mut DaemonState, paths: &[String], _start_idx: u64) -> Vec<TrackInfo> {
    let mut tracks = Vec::with_capacity(paths.len());
    for path in paths {
        tracks.push(resolve_track(path));
    }
    state.queue = tracks;
    state.queue_cursor = 0;
    state.default_list.clear();
    state.default_cursor = 0;
    state.fallback_disabled = false;
    state.queue.clone()
}

/// Clear the user queue and the default-list session.  Disables the
/// auto-build fallback so playback stops after the current track ends.
pub fn queue_clear(state: &mut DaemonState) {
    state.queue.clear();
    state.queue_cursor = 0;
    state.default_list.clear();
    state.default_cursor = 0;
    state.fallback_disabled = true;
}

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus", "wma"];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

/// Expand a list of user-supplied paths into concrete audio files. A path
/// that resolves to a directory is scanned recursively; an existing file is
/// kept only if it has an audio extension. Missing paths are queued as-is
/// (playback reports the failure), preserving the historical tolerant
/// behaviour for paths that may not exist yet.
pub fn expand_paths(paths: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for path in paths {
        let p = Path::new(path);
        if p.is_dir() {
            out.extend(scan_audio_files(path));
        } else if p.is_file() {
            if !is_audio_file(p) {
                return Err(format!("not an audio file: {path}"));
            }
            out.push(path.clone());
        } else {
            out.push(path.clone());
        }
    }
    Ok(out)
}

pub fn scan_audio_files(path: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        if dir.is_file() && is_audio_file(dir) {
            paths.push(path.to_string());
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
        if is_audio_file(entry.path()) {
            paths.push(entry.path().to_string_lossy().to_string());
        }
    }
    paths
}
