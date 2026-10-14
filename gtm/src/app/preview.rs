// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Hover preview: audition the radio or chart row the cursor rests on
//
// This is free software released under the GPL-3.0 license.

//! Sound preview for a resting cursor.
//!
//! The radio directory lists stations and a chart lists tracks; neither can be
//! judged from its name, so resting on a row for a moment starts a preview. The
//! timer is re-armed whenever the highlighted row changes, so only a genuine
//! rest fires, and the preview is skipped whenever something is already
//! playing: a row that cut off the track the user chose would be the opposite
//! of an audition.

use crate::app::*;
use crate::providers::charts::ChartTrack;

/// What a resting cursor offers to play.
enum PreviewKind {
    Radio { id: String, name: String },
    Chart(ChartTrack),
}

/// A resting row: its stable key (for re-arming) and what to play.
struct PreviewTarget {
    key: String,
    kind: PreviewKind,
}

impl App {
    /// Advance the hover preview. Called every iteration of the frame loop: it
    /// re-arms the timer when the cursor moves to another row, then starts the
    /// preview once the configured delay has elapsed with nothing else playing.
    pub(crate) fn poll_preview(&mut self) {
        // A peek is only owed while the radio picker is the thing on screen to
        // close. Every other way out of it — a command that closes the picker
        // directly, a picker opened over the top — ends the borrow as surely as
        // Esc does, and a station left sounding under a picker that is no longer
        // there has nothing left to give it back.
        if self.peek_station.is_some() && !self.pickers.has(PickerId::Radio) {
            self.peek_end();
        }
        let target = self.preview_target();
        let key = target.as_ref().map(|t| t.key.clone());
        if key != self.preview_hover {
            self.preview_hover = key;
            self.preview_since = Some(std::time::Instant::now());
            self.preview_fired = false;
            return;
        }
        let Some(target) = target else {
            return;
        };
        if self.preview_fired || self.preview_delay_secs == 0 {
            return;
        }
        let Some(since) = self.preview_since else {
            return;
        };
        if since.elapsed() < std::time::Duration::from_secs(self.preview_delay_secs) {
            return;
        }
        // "Nothing else playing" is read from the transport, not from who
        // started the sound, so a preview never cuts off a track the user chose.
        // Inside the radio picker the rest is a *peek*, which may also borrow a
        // paused player and is handed back when the picker closes.
        let peek = self.pickers.top().is_some_and(|o| o.id == PickerId::Radio);
        if peek {
            if !self.peekable() {
                return;
            }
        } else if self.state.status != PlaybackStatus::Stopped {
            return;
        }
        self.preview_fired = true;
        match target.kind {
            PreviewKind::Radio { id, name } => {
                if peek {
                    self.peek_play(&id, &name);
                } else {
                    let c = self.client.clone();
                    tokio::spawn(async move {
                        let _ = c.radio().play(&id, &name).await;
                    });
                }
            }
            PreviewKind::Chart(track) => {
                let c = self.client.clone();
                let ipc = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = Self::play_chart(&c, track).await {
                        let _ = ipc.send(IpcResult::Error(format!("Preview: {e}")));
                    }
                });
            }
        }
    }

    /// The row under the cursor that can be auditioned, if any.
    ///
    /// Only the radio directory (the picker or the category) and an opened
    /// chart's tracks name something playable; every other list answers `None`,
    /// which disarms the preview.
    fn preview_target(&self) -> Option<PreviewTarget> {
        // A picker on top owns the cursor. The radio directory is the one that
        // offers stations; any other picker disarms the preview.
        if let Some(top) = self.pickers.top() {
            if top.id != PickerId::Radio {
                return None;
            }
            return self.radio_picker_target(top.selected);
        }
        if self.library_pane_focus {
            return None;
        }
        match self.library_category {
            LIB_RADIO => self.custom_station_target(self.list_pos()),
            LIB_CHARTS => self.chart_track_target(self.list_pos()),
            _ => None,
        }
    }

    fn radio_picker_target(&self, selected: usize) -> Option<PreviewTarget> {
        match self.radio_picks().get(selected) {
            Some(RadioPick::Custom(i)) => self.custom_station_target(*i),
            Some(RadioPick::Station(i)) => {
                let stations = match self.radio.section {
                    RadioSection::Stations => &self.radio.browse_stations,
                    RadioSection::Results => &self.radio.search,
                    RadioSection::Root => &self.radio.top,
                };
                stations.get(*i).map(|s| PreviewTarget {
                    key: format!("radio:{}", s.id),
                    kind: PreviewKind::Radio {
                        id: s.id.clone(),
                        name: s.name.clone(),
                    },
                })
            }
            _ => None,
        }
    }

    fn custom_station_target(&self, i: usize) -> Option<PreviewTarget> {
        self.radio.custom.get(i).map(|s| {
            let id = s
                .uuid
                .clone()
                .unwrap_or_else(|| format!("custom:{}", i + 1));
            PreviewTarget {
                key: format!("radio:{id}"),
                kind: PreviewKind::Radio {
                    id,
                    name: s.name.clone(),
                },
            }
        })
    }

    fn chart_track_target(&self, i: usize) -> Option<PreviewTarget> {
        let chart = self.charts.selected_chart?;
        self.charts.chart_tracks.get(i).map(|t| PreviewTarget {
            key: format!("chart:{chart}|{}|{}", t.uri, t.title),
            kind: PreviewKind::Chart(t.clone()),
        })
    }
}

/// What a peek displaced, held until the picker closes.
///
/// The transport has no single verb for "load this and leave it paused", so
/// restoring a pause takes two; remembering that it *was* a pause is what makes
/// the difference between browsing and accidentally listening.
pub(crate) struct PeekRestore {
    pub(crate) path: String,
    pub(crate) pos: f64,
    pub(crate) paused: bool,
}

/// Peeking into a station: sounding it without leaving the row it was found on.
///
/// A peek differs from the library list's hover preview in two ways. It may
/// borrow a *paused* player — the user stopped to browse, and putting the music
/// back afterwards is expected rather than rude — so it keeps what it displaced.
/// And it is undone explicitly, when the picker closes, rather than fading out
/// on its own.
impl App {
    /// Whether the transport is free for a peek to borrow.
    ///
    /// Playing audio over an audible track would cut it off, so only silence
    /// and a pause qualify. A pause is remembered so the peek can give it back.
    ///
    /// A player that is already sounding *is* free when the sound is a previous
    /// peek's: moving the cursor to another station is the point of peeking, and
    /// the station it replaces is one the user only ever asked to hear. Without
    /// this the first peek made the transport unpeekable, so the cursor could
    /// change but the music never could.
    fn peekable(&mut self) -> bool {
        match self.state.status {
            PlaybackStatus::Stopped => true,
            PlaybackStatus::Paused => {
                let Some(track) = self.state.current_track.as_ref() else {
                    return false;
                };
                self.peek_restore = Some(PeekRestore {
                    path: track.path.clone(),
                    pos: self.state.time_pos,
                    paused: true,
                });
                true
            }
            PlaybackStatus::Playing => self.sounding_peek(),
        }
    }

    /// Whether the station on air is the one an outstanding peek started.
    ///
    /// The question every peek decision turns on: a peek owns the transport only
    /// while what is sounding is what it started, and this says so without the
    /// callers having to know how a station is spelled in a track path.
    pub(crate) fn sounding_peek(&self) -> bool {
        let Some(id) = self.peek_station.as_deref() else {
            return false;
        };
        // A station is queued as `radio://<id>` or `radio://<id>/<name>`, and the
        // id is the whole of it — so that is what is compared, rather than the
        // whole path, which the stream's own title can change under us.
        self.state
            .current_track
            .as_ref()
            .and_then(|t| t.path.strip_prefix("radio://"))
            .and_then(|rest| rest.split('/').next())
            .is_some_and(|sounding| sounding == id)
    }

    /// Begin a peek of `id`, unless that station is the one already sounding.
    pub(crate) fn peek_play(&mut self, id: &str, name: &str) {
        if self.peek_station.as_deref() == Some(id) {
            return;
        }
        self.peek_station = Some(id.to_string());
        let c = self.client.clone();
        let (id, name) = (id.to_string(), name.to_string());
        tokio::spawn(async move {
            let _ = c.radio().play(&id, &name).await;
        });
    }

    /// Forget an outstanding peek without touching the transport.
    ///
    /// The other half of [`Self::peek_end`]: for when playback has moved on by
    /// somebody else's decision — the user picked a station of their own, a track
    /// auto-advanced — so handing the transport back would stop or rewind what
    /// they actually asked for.
    pub(crate) fn peek_drop(&mut self) {
        self.peek_station = None;
        self.peek_restore = None;
    }

    /// Put the transport back the way a peek found it.
    ///
    /// A peek that displaced a *paused* track brings it back where it stopped
    /// and leaves it paused: the user paused it, and a peek that resumed it on
    /// the way out would have turned browsing into listening. A peek that
    /// started from silence just stops. Idempotent, so closing the picker twice
    /// is harmless.
    pub(crate) fn peek_end(&mut self) {
        // A peek is only a peek while the thing sounding is the thing it
        // started. Anything else — the user chose a station of their own, a
        // track auto-advanced — means the transport has moved on, and stopping
        // it here would silence the music they are actually listening to.
        let owned = self.sounding_peek();
        if self.peek_station.take().is_none() {
            return;
        }
        if !owned {
            self.peek_restore = None;
            return;
        }
        let c = self.client.clone();
        match self.peek_restore.take() {
            Some(r) => tokio::spawn(async move {
                let _ = c.play(&r.path, r.pos).await;
                // The daemon has no "load without playing", so the pause is
                // reapplied right after. It only takes effect if the play
                // landed, which is why it is not fired unconditionally.
                if r.paused {
                    let _ = c.pause().await;
                }
            }),
            None => tokio::spawn(async move {
                let _ = c.stop().await;
            }),
        };
    }
}
