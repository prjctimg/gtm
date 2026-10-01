// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Music browse: search, album tracklists, artist pages
//
// This is free software released under the GPL-3.0 license.

//! The answer to "who is this, and what else is there" — the question Spotify
//! can no longer be asked. Its artist-contents endpoints were removed for
//! developer-mode integrations, so there is no request to make for an artist's
//! releases or an album's tracklist; Deezer's keyless public API supplies both
//! and is already a chart provider here.

pub mod app;
pub mod picker;
