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
        if self.state.status != PlaybackStatus::Stopped {
            return;
        }
        self.preview_fired = true;
        match target.kind {
            PreviewKind::Radio { id, name } => {
                let c = self.client.clone();
                tokio::spawn(async move {
                    let _ = c.radio().play(&id, &name).await;
                });
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
            let id = s.uuid.clone().unwrap_or_else(|| format!("custom:{}", i + 1));
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
