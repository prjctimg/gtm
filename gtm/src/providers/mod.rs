// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Provider integrations in the client: one directory per external service
//
// This is free software released under the GPL-3.0 license.

//! The client half of the provider split: each integration's wire types, its
//! state, its actions and its picker rendering live together. The
//! cross-provider concerns — the IPC enum, the key dispatcher, the cover and
//! lyrics routers — stay in `shared`, `app` and `ui`, because each of them
//! fans out over every provider and belongs to none.

pub mod browse;
pub mod charts;
pub mod lastfm;
pub mod podcast;
pub mod radio;
pub mod spotify;
pub mod yt;
