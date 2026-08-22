// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Mouse hit-testing: the UI layer registers clickable row rectangles each
// frame and the event loop resolves clicks against them (PROMPT #7).
//
// This is free software released under the GPL-3.0 license.

use ratatui::layout::Rect;
use std::time::{Duration, Instant};

/// A registered clickable region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseZone {
    /// Row `usize` inside the topmost picker's list (absolute item index,
    /// matching what picker navigation uses for `selected`).
    PickerItem(usize),
    /// Row in the focused main-pane list (absolute index into that list).
    ListItem(usize),
}

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// Per-frame map of clickable rectangles, rebuilt by `ui::render`.
#[derive(Default)]
pub struct MouseMap {
    zones: Vec<(Rect, MouseZone)>,
    /// Full rect of the topmost picker panel; clicks outside it close the
    /// picker.
    pub picker_area: Option<Rect>,
    last_click: Option<(MouseZone, Instant)>,
}

impl MouseMap {
    pub fn clear(&mut self) {
        self.zones.clear();
        self.picker_area = None;
        // last_click intentionally survives: double-clicks span frames.
    }

    pub fn register(&mut self, rect: Rect, zone: MouseZone) {
        if rect.width > 0 && rect.height > 0 {
            self.zones.push((rect, zone));
        }
    }

    pub fn set_picker_area(&mut self, rect: Rect) {
        self.picker_area = Some(rect);
    }

    /// Resolve the zone under a click. Later registrations win so overlay
    /// rows shadow anything beneath them.
    pub fn hit_test(&self, x: u16, y: u16) -> Option<MouseZone> {
        self.zones
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
            .map(|(_, z)| *z)
    }

    /// Record a click on `zone`; returns true when this is the second press
    /// on the same zone inside the double-click window.
    pub fn is_double_click(&mut self, zone: MouseZone) -> bool {
        let now = Instant::now();
        let double = self
            .last_click
            .take()
            .is_some_and(|(z, t)| z == zone && now.duration_since(t) <= DOUBLE_CLICK_WINDOW);
        if !double {
            self.last_click = Some((zone, now));
        }
        double
    }
}
