// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Provider integrations: one directory per external service
//
// This is free software released under the GPL-3.0 license.

//! Every external music service the daemon talks to lives under its own
//! directory, so a single integration can be read, reviewed and replaced
//! without touching the others. Each provider owns its client, its artwork and
//! lyric lookups, and its streaming bridge; the shared on-disk stores
//! ([`crate::cover`] and [`crate::lrclib`]) stay outside because more than one
//! provider feeds them.

pub mod browse;
pub mod charts;
pub mod deezer;
pub mod lastfm;
pub mod lrclib;
pub mod musicbrainz;
pub mod podcast;
pub mod radio;
pub mod spotify;
#[cfg(feature = "youtube")]
pub mod youtube;
