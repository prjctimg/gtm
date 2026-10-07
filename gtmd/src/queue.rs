// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Queue management: add, remove, move, clear
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;

use gtm::shared::global::DaemonState;
use gtm::shared::state::PlaybackStatus;
use gtm::shared::track::TrackInfo;

use crate::library::extract_metadata;

/// Build a TrackInfo from a file path.  The title is derived from
/// the file stem; all other fields are left empty/default.  The path is
/// canonicalised so path-equality checks against `daemon::resolve_track_meta`
/// results (queue consumption, Play/Prev tracking) stay consistent.
///
/// When the file carries audio tags they are read so queued entries show
/// clean metadata (title/artist/album) instead of the raw filename.  Untagged
/// or unprobeable files fall back to the file stem.
pub fn resolve_track(path: &str) -> TrackInfo {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| Path::new(path).to_path_buf());
    let path_str = canonical.to_string_lossy().into_owned();
    let stem = canonical
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Remote provider URIs (e.g. `spotify:track:...`) have no filesystem
    // metadata: label them by provider instead of surfacing the raw URI in
    // the queue / now-playing widget. Streaming code (queue_stream, remote
    // resolve) overwrites this with the real title once playback starts.
    if path_str.starts_with("spotify:") {
        return TrackInfo {
            id: 0,
            title: gtm::shared::spotify::pretty_id(&path_str),
            path: path_str,
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
        };
    }

    if let Ok((meta, hash)) = extract_metadata(&path_str, None) {
        return TrackInfo {
            id: 0,
            path: path_str,
            title: if meta.title.is_empty() {
                stem.clone()
            } else {
                meta.title
            },
            artist: if meta.artist.is_empty() {
                String::new()
            } else {
                meta.artist
            },
            album: if meta.album.is_empty() {
                String::new()
            } else {
                meta.album
            },
            duration: meta.duration,
            track_number: meta.track_number,
            genre: meta.genre,
            year: meta.year,
            bitrate: meta.bitrate,
            samplerate: meta.samplerate,
            hash,
            cover_path: meta.cover_path,
            favourite: false,
            ..Default::default()
        };
    }

    TrackInfo {
        id: 0,
        path: path_str,
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
pub fn visible(state: &DaemonState) -> (Vec<TrackInfo>, u64) {
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

/// Whether `path` is already in the queue, as `(is_user, local)`.
///
/// Path is the only identity available: every provider entry — Spotify,
/// podcast, radio, a chart row — is queued with `id == 0`, so an id-based
/// comparison would call every provider track the same track.
fn find_path(state: &DaemonState, path: &str) -> Option<(bool, usize)> {
    if let Some(i) = state.queue.iter().position(|t| t.path == path) {
        return Some((true, i));
    }
    state
        .default_list
        .iter()
        .position(|t| t.path == path)
        .map(|i| (false, i))
}

/// Insert a track into the merged view at `pos`, maintaining the cursor.
///
/// Returns `false` when the path is already queued, in which case nothing is
/// inserted. There was no duplicate check anywhere: adding a track that was
/// already queued, adding a playlist that overlapped the queue, or pressing
/// `a` twice on a row all produced a second copy — and because `Cmd::play`
/// rotates by *path*, the two copies were indistinguishable afterwards, so
/// playing one left the other as a phantom row.
fn insert_at(state: &mut DaemonState, track: TrackInfo, pos: usize) -> bool {
    if find_path(state, &track.path).is_some() {
        return false;
    }
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
    true
}

/// Add a track.  `position == None` queues it to play next (right after the
/// current entry); `Some(pos)` inserts at an explicit merged-view index.
/// Returns the entry now in the queue, which for a path that was already
/// queued is the pre-existing copy rather than a new one.
pub fn add(state: &mut DaemonState, path: &str, position: Option<u64>) -> TrackInfo {
    let mut added = add_many(state, &[path.to_string()], position);
    added.pop().expect("add_many returns one entry per path")
}

/// Index the next inserted entry lands at, given an optional explicit
/// merged-view position. Shared by every insert path so they cannot drift.
fn insert_base(state: &DaemonState, position: Option<u64>) -> usize {
    match position {
        Some(p) => {
            let len = state.queue.len() + state.default_list.len();
            (p as usize).min(len)
        }
        None => {
            if state.queue.is_empty() {
                0
            } else {
                1
            }
        }
    }
}

/// Add multiple tracks as a batch.  The whole batch is queued to play next
/// (after the current entry) unless `position` is given, preserving order.
pub fn add_many(
    state: &mut DaemonState,
    paths: &[String],
    position: Option<u64>,
) -> Vec<TrackInfo> {
    let mut added = Vec::with_capacity(paths.len());
    let mut at = insert_base(state, position);
    for path in paths {
        added.push(place(state, resolve_track(path), &mut at, position));
    }
    added
}

/// Insert `track` at `at`, advancing `at` only if it was actually inserted, and
/// return the entry that is now in the queue — the new one, or the pre-existing
/// copy when the path was already queued.
///
/// The insert position has to track *successful* inserts rather than the loop
/// index: a skipped duplicate would otherwise leave every later track one slot
/// too far along.
fn place(
    state: &mut DaemonState,
    track: TrackInfo,
    at: &mut usize,
    position: Option<u64>,
) -> TrackInfo {
    if insert_at(state, track.clone(), *at) {
        if position.is_none() {
            *at += 1;
        }
        return track;
    }
    let path = track.path.clone();
    match find_path(state, &path) {
        Some((true, i)) => state.queue[i].clone(),
        Some((false, i)) => state.default_list[i].clone(),
        None => track,
    }
}

/// Insert a pre-resolved track using the same merged-view placement as
/// [`add_many`]. Metadata gathering happens before the `DaemonState` write
/// lock is taken, so the insert itself stays free of disk I/O and tag reads.
/// Returns the entry now in the queue, which for an already-queued path is the
/// pre-existing copy.
pub fn add_resolved(state: &mut DaemonState, track: TrackInfo, position: Option<u64>) -> TrackInfo {
    let mut at = insert_base(state, position);
    place(state, track, &mut at, position)
}

/// Insert a batch of pre-resolved tracks, preserving order.
///
/// A provider that already holds every track's metadata inserts it here rather
/// than adding by path and patching afterwards: the patch has to find the entry
/// by path, so a playlist listing the same track twice writes both copies'
/// metadata onto the first one and leaves the second as a bare placeholder.
/// Duplicates within the batch and against the existing queue are skipped.
pub fn add_resolved_many(state: &mut DaemonState, tracks: Vec<TrackInfo>, position: Option<u64>) {
    let mut at = insert_base(state, position);
    for track in tracks {
        place(state, track, &mut at, position);
    }
}

/// Replace the user queue with pre-resolved tracks and drop the default-list
/// session, mirroring [`set`] without re-reading tags.
pub fn set_resolved(state: &mut DaemonState, tracks: Vec<TrackInfo>) {
    state.queue = tracks;
    state.queue_cursor = 0;
    state.default_list.clear();
    state.default_cursor = 0;
    state.fallback_disabled = false;
    // Whatever was playing is now in neither the queue nor the library, so
    // now-playing names a row the user cannot reach. The caller sends `Set`
    // immediately before `Play` for the row it meant to start, which usually
    // corrects this microseconds later -- so the invariant held by accident, and
    // a `Play` that failed left the daemon reporting a track from a list that no
    // longer exists.
    if let Some(cur) = state.current_track.clone()
        && !state.queue.iter().any(|t| t.path == cur.path)
    {
        state.current_track = None;
        state.time_pos = 0.0;
        state.status = PlaybackStatus::Stopped;
    }
}

/// Remove the entry at a merged-view index.  Returns the removed track, or
/// None if the index is out of range.
pub fn remove(state: &mut DaemonState, index: u64) -> Option<TrackInfo> {
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
pub fn move_track(state: &mut DaemonState, from: u64, to: u64) -> bool {
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
    // `to` is already a merged-view index and `insert_at` reads it as one.
    // It used to be converted to a *local* index first, which then had the
    // queue length subtracted from it a second time inside `insert_at` — so a
    // move into the default-list region landed `ulen` slots too early. With a
    // queue of 2 over a default list of 4, moving row 0 to row 5 produced
    // `B C D A E F` instead of `B C D E F A`.
    insert_at(state, track, to);
    true
}

/// Clear the user queue and the default-list session.  Disables the
/// auto-build fallback so playback stops after the current track ends.
pub fn clear(state: &mut DaemonState) {
    state.queue.clear();
    state.queue_cursor = 0;
    state.default_list.clear();
    state.default_cursor = 0;
    state.fallback_disabled = true;
}

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus"];

/// Audio that `wma` used to sit beside in [`AUDIO_EXTENSIONS`] and that nothing
/// can decode.
///
/// Symphonia's `all-codecs` is aac, adpcm, alac, flac, mp1, mp2, mp3, pcm and
/// vorbis; there is no WMA decoder in it or anywhere else in the build. Listing
/// the extension anyway meant `queue-add` took the file, reported success, and
/// the failure only surfaced once playback reached it. Rejecting it here says
/// "unsupported" instead of letting it look playable.
const UNDECODABLE_EXTENSIONS: &[&str] = &["wma"];

fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
}

fn is_audio_file(path: &Path) -> bool {
    ext_of(path).is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
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
            // Audio we cannot decode is reported as unsupported rather than as
            // "not an audio file": it is audio, and the distinction is the whole
            // answer.
            if let Some(ext) = ext_of(p)
                && UNDECODABLE_EXTENSIONS.contains(&ext.as_str())
            {
                return Err(format!(
                    "unsupported audio format (nothing here can decode .{ext}): {path}"
                ));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, title: &str) -> TrackInfo {
        TrackInfo {
            path: path.to_string(),
            title: title.to_string(),
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

    /// A track already queued is not queued twice.
    ///
    /// Nothing checked this: adding a queued track, adding a playlist that
    /// overlapped the queue, or pressing `a` twice on a row all left a second
    /// copy behind. Identity is the path, because every provider entry is
    /// queued with `id == 0`.
    #[test]
    fn an_already_queued_path_is_not_duplicated() {
        let mut state = DaemonState::default();
        let uri = "spotify:track:4cOdK2wGLETKBW3PvgPWqT";
        add_resolved(&mut state, track(uri, "Song"), None);
        add_resolved(&mut state, track(uri, "Song"), None);
        assert_eq!(state.queue.len(), 1, "the second add must be a no-op");
        // The pre-existing copy is returned, so a caller that wants to play it
        // still gets a usable entry.
        assert_eq!(
            add_resolved(&mut state, track(uri, "Song"), None).title,
            "Song"
        );
    }

    /// The same rule holds within a single batch, and the skip must not shift
    /// the tracks that follow it.
    #[test]
    fn a_duplicate_inside_a_batch_does_not_shift_the_rest() {
        let mut state = DaemonState::default();
        add_resolved_many(
            &mut state,
            vec![
                track("a", "A"),
                track("b", "B"),
                track("a", "A again"),
                track("c", "C"),
            ],
            None,
        );
        let paths: Vec<&str> = state.queue.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["a", "b", "c"], "C must land third, not fourth");
    }

    /// A path in the default list also counts as already queued — the merged
    /// view is one list to the user.
    #[test]
    fn a_default_list_entry_is_not_duplicated_into_the_queue() {
        let mut state = DaemonState {
            default_list: vec![track("/music/song.mp3", "Song")],
            ..Default::default()
        };
        add_resolved(&mut state, track("/music/song.mp3", "Song"), None);
        assert!(state.queue.is_empty(), "must not shadow the library entry");
    }

    /// Batch order survives the insert, which is what `shuffle` ordering depends
    /// on.
    #[test]
    fn batch_keeps_order() {
        let mut state = DaemonState::default();
        add_resolved_many(
            &mut state,
            vec![track("spotify:track:a", "A"), track("spotify:track:b", "B")],
            None,
        );
        let paths: Vec<&str> = state.queue.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["spotify:track:a", "spotify:track:b"]);
    }

    /// A move across the user-queue/default-list boundary lands where it was
    /// asked to.
    ///
    /// The target index used to be converted to a *local* index and then had the
    /// queue length subtracted from it again inside `insert_at`, so moving row 0
    /// to the end of a 2-track queue over a 4-track library produced
    /// `B C D A E F` instead of `B C D E F A`.
    #[test]
    fn a_move_into_the_default_region_lands_where_asked() {
        let mut state = DaemonState {
            queue: vec![track("a", "A"), track("b", "B")],
            default_list: vec![
                track("c", "C"),
                track("d", "D"),
                track("e", "E"),
                track("f", "F"),
            ],
            ..Default::default()
        };
        assert!(move_track(&mut state, 0, 5), "move should succeed");
        let (merged, _) = visible(&state);
        let paths: Vec<&str> = merged.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["b", "c", "d", "e", "f", "a"]);
    }

    /// A move within the user queue is unaffected.
    #[test]
    fn a_move_inside_the_queue_lands_where_asked() {
        let mut state = DaemonState {
            queue: vec![track("a", "A"), track("b", "B"), track("c", "C")],
            ..Default::default()
        };
        assert!(move_track(&mut state, 2, 0));
        let paths: Vec<&str> = state.queue.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, ["c", "a", "b"]);
    }

    /// A provider URI has no file to read a title from, so the queue must never
    /// show the raw URI.
    #[test]
    fn provider_uri_is_labelled_not_printed() {
        let t = resolve_track("spotify:track:4cOdK2wGLETKBW3PvgPWqT");
        assert_eq!(t.title, "Spotify Track");
        assert!(!t.title.contains("spotify:"));
    }

    /// A provider URI is a resource, not a filesystem path, so the queue route
    /// has to recognise it and hand it to a resolver instead of a tag read.
    #[test]
    fn provider_paths_are_recognised() {
        for p in [
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "podcast://feed/2",
            "radio://abc",
            "youtube:xyz",
        ] {
            assert!(crate::daemon::is_provider_path(p), "{p}");
        }
        for p in ["/music/song.mp3", "song.mp3", "/music", ""] {
            assert!(!crate::daemon::is_provider_path(p), "{p}");
        }
    }
}
