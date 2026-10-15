// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Hover preview: audition the radio or chart row the cursor rests on
//
// This is free software released under the GPL-3.0 license.

//! Sound preview for a resting cursor.
//!
//! A chart lists tracks that cannot be judged from their name, so resting on a
//! row for a moment starts one. The timer is re-armed whenever the highlighted
//! row changes, so only a genuine rest fires, and the preview is skipped
//! whenever something is already playing: a row that cut off the track the user
//! chose would be the opposite of an audition. The radio directory describes
//! its rows in a preview strip instead — see `render_radio`.

use crate::app::*;
use crate::providers::charts::ChartTrack;

/// What a resting cursor offers to play.
type PreviewKind = ChartTrack;

/// A resting row: its stable key (for re-arming) and what to play.
struct PreviewTarget {
    key: String,
    kind: PreviewKind,
}

impl App {
    /// Advance the hover preview. Called every iteration of the frame loop: it
    /// re-arms the timer when the cursor moves to another row, then starts the
    /// preview once the configured delay has elapsed with nothing else playing.
    ///
    /// Chart tracks only. A station used to be auditioned the same way, inside
    /// the radio picker, by borrowing the transport — which meant resting on a
    /// row rewrote now-playing, displaced whatever the user had paused, and
    /// needed a hand-back on every way out of the picker, ~30 of which closed
    /// the picker without going through the cleanup. The radio picker shows the
    /// station's details in a preview strip instead: nothing is borrowed, so
    /// there is nothing to restore and no way to strand playback.
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
        // "Nothing else playing" is read from the transport, so a preview never
        // cuts off a track the user chose.
        if self.state.status != PlaybackStatus::Stopped {
            return;
        }
        self.preview_fired = true;
        let c = self.client.clone();
        let ipc = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::play_chart(&c, target.kind).await {
                let _ = ipc.send(IpcResult::Error(format!("Preview: {e}")));
            }
        });
    }

    /// The row under the cursor that can be auditioned, if any.
    ///
    /// Only the radio directory (the picker or the category) and an opened
    /// chart's tracks name something playable; every other list answers `None`,
    /// which disarms the preview.
    fn preview_target(&self) -> Option<PreviewTarget> {
        // A picker on top owns the cursor, and no picker previews anything now.
        if self.pickers.is_open() || self.library_pane_focus {
            return None;
        }
        match self.library_category {
            LIB_CHARTS => self.chart_track_target(self.list_pos()),
            _ => None,
        }
    }

    fn chart_track_target(&self, i: usize) -> Option<PreviewTarget> {
        let chart = self.charts.selected_chart?;
        self.charts.chart_tracks.get(i).map(|t| PreviewTarget {
            key: format!("chart:{chart}|{}|{}", t.uri, t.title),
            kind: t.clone(),
        })
    }
}
