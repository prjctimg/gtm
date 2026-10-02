use crate::app::*;

/// Actions for the unified presentation picker.
///
/// Every value is applied as the cursor moves rather than on Enter. That is what
/// makes the preview worth drawing: the app behind the panel already *is* the
/// value under the cursor, so the drawing is a sample of the real thing instead
/// of a picture of it. Each of the five is a single assignment, which is why
/// this is safe to do per keystroke.
impl App {
    /// The highlighted row: the picker's own selection, which the mouse and
    /// `clamp_picker_selection` already move.
    pub(crate) fn look_row(&self) -> usize {
        self.pickers.top().map_or(0, |o| o.selected)
    }

    /// Put the cursor on `row`.
    fn look_set_row(&mut self, row: usize) {
        let max = self.look_count().saturating_sub(1);
        if let Some(top) = self.pickers.top_mut() {
            top.selected = row.min(max);
        }
    }

    /// Put the cursor back on the first row, which is what a changed filter
    /// means: the old row names a value the list no longer shows.
    pub(crate) fn look_row_reset(&mut self) {
        self.look_set_row(0);
    }

    /// Index of the value the current category is set to.
    fn look_current(&self) -> usize {
        match self.look.cat() {
            Look::Layout => match self.zen_surface {
                ZenSurface::Lyrics => 0,
                ZenSurface::NowPlaying => 1,
                ZenSurface::Visualizer => 2,
            },
            Look::Theme => self.theme_index.min(self.themes.len().saturating_sub(1)),
            Look::Visualizer => VisualizerPreset::all()
                .iter()
                .position(|p| *p == self.visualizer.preset)
                .unwrap_or(0),
            Look::Progress => ProgressStyle::all()
                .iter()
                .position(|p| *p == self.progress_style)
                .unwrap_or(0),
            Look::Footer => self
                .footer_preset
                .min(self.footer_presets.len().saturating_sub(1)),
        }
    }

    /// The highlighted row as an index into whatever backs the category.
    ///
    /// Every category except Theme lists its source in order, so the row *is*
    /// the index. Themes filter by name, so the row is a position in the
    /// filtered list and has to be resolved — a theme name and a filtered row
    /// number are not the same number, and treating them as one would apply
    /// whichever theme happened to sit at row 3 of whatever was typed.
    fn look_source_index(&self) -> usize {
        let row = self.look_row();
        if self.look.cat() != Look::Theme {
            return row;
        }
        let q = self.look.query.to_lowercase();
        self.themes
            .iter()
            .enumerate()
            .filter(|(_, t)| q.is_empty() || t.name.to_lowercase().contains(&q))
            .nth(row)
            .map(|(i, _)| i)
            .unwrap_or(row)
    }

    /// Apply the highlighted value.
    ///
    /// This is both the preview and the commit: there is no second, deferred
    /// apply, because the whole point of the panel is that the app behind it is
    /// already showing the value under the cursor.
    pub(crate) fn look_preview(&mut self) {
        let cat = self.look.cat();
        let i = self.look_source_index();
        match cat {
            Look::Layout => {
                let surfaces = [
                    ZenSurface::Lyrics,
                    ZenSurface::NowPlaying,
                    ZenSurface::Visualizer,
                ];
                if let Some(s) = surfaces.get(i).copied() {
                    self.zen_surface = s;
                }
            }
            Look::Theme => self.apply_theme_index(i),
            Look::Visualizer => {
                if let Some(p) = VisualizerPreset::all().get(i) {
                    self.visualizer.preset = *p;
                }
                save_prefs(&self.current_prefs());
            }
            Look::Progress => {
                if let Some(p) = ProgressStyle::all().get(i) {
                    self.progress_style = *p;
                }
                save_prefs(&self.current_prefs());
            }
            Look::Footer => self.apply_preset_index(i),
        }
    }

    /// Move the cursor by `d`, previewing on the way.
    pub(crate) fn look_move(&mut self, d: isize) {
        let count = self.look_count();
        if count == 0 {
            return;
        }
        let next = (self.look_row() as isize + d).rem_euclid(count as isize) as usize;
        self.look_set_row(next);
        self.look_preview();
    }

    /// Switch category and land the cursor on what that category is set to, so
    /// the picker never opens on a value the user has already replaced.
    pub(crate) fn look_switch(&mut self, forward: bool) {
        self.look.cycle(forward);
        self.look_set_row(self.look_current());
        self.look_preview();
    }

    /// Open the picker on a particular category, with its cursor on the current
    /// value.
    ///
    /// The keys that used to open each picker on its own still do, so a keypress
    /// that named a setting lands on that setting rather than on the first one.
    pub(crate) fn open_look_on(&mut self, cat: Look) {
        self.look.cat = Some(cat);
        self.look.query.clear();
        self.pickers.open(PickerId::Look);
        self.look_set_row(self.look_current());
        self.on_picker_opened(PickerId::Look);
    }

    /// Whether the top picker's keys are text rather than navigation.
    ///
    /// `j` and `k` only move the cursor in a picker with nothing to type into,
    /// and the Look picker is the one case that differs *within* itself: themes
    /// filter by name, so their keys are text, while the other four categories
    /// are short lists where `j` and `k` have to stay navigation. Deciding once,
    /// here, is what keeps `k` from typing a `k` into the visualizer list.
    pub(crate) fn picker_takes_text(&self) -> bool {
        match self.pickers.top().map(|o| o.id) {
            Some(PickerId::YTSearch)
            | Some(PickerId::SearchLibrary)
            | Some(PickerId::CommandPalette)
            | Some(PickerId::SpotifySearch)
            | Some(PickerId::SpotifyLink)
            | Some(PickerId::SpotifyDest) => true,
            Some(PickerId::Look) => self.look.cat() == Look::Theme,
            _ => false,
        }
    }

    /// Preview whatever a picker's cursor move should preview.
    ///
    /// Only the Look picker previews: the others either have nothing to preview
    /// or fetch artwork, and routing every picker's cursor through one function
    /// is how the two used to diverge.
    pub(crate) fn picker_nav_preview(&mut self) {
        if self.pickers.top().is_some_and(|o| o.id == PickerId::Look) {
            self.look_preview();
        }
    }

    /// Say what is set now, so Enter can close the picker with a confirmation.
    ///
    /// `look_preview` has already applied the value by the time this runs; all
    /// that is left is the name and one last save.
    pub(crate) fn look_apply(&mut self) -> String {
        save_prefs(&self.current_prefs());
        match self.look.cat() {
            Look::Layout => {
                let name = match self.zen_surface {
                    ZenSurface::Lyrics => "Lyrics",
                    ZenSurface::NowPlaying => "Now Playing",
                    ZenSurface::Visualizer => "Visualizer",
                };
                format!("Zen: {name}")
            }
            Look::Theme => {
                let t = self.themes.get(self.theme_index);
                let name = t.map(|t| t.name.to_string()).unwrap_or_default();
                let light = if t.is_some_and(|t| t.light) {
                    " (light)"
                } else {
                    ""
                };
                format!("Theme: {name}{light}")
            }
            Look::Visualizer => format!("Visualizer: {}", self.visualizer.preset.name()),
            Look::Progress => format!("Progress: {}", self.progress_style.name()),
            Look::Footer => {
                let n = self
                    .footer_presets
                    .get(self.footer_preset)
                    .map(|p| p.name.to_string())
                    .unwrap_or_else(|| "Default".into());
                format!("Footer: {n}")
            }
        }
    }

    /// How many values the current category offers.
    pub(crate) fn look_count(&self) -> usize {
        match self.look.cat() {
            Look::Layout => 3,
            Look::Theme => {
                let q = self.look.query.to_lowercase();
                self.themes
                    .iter()
                    .filter(|t| q.is_empty() || t.name.to_lowercase().contains(&q))
                    .count()
            }
            Look::Visualizer => VisualizerPreset::all().len(),
            Look::Progress => ProgressStyle::all().len(),
            Look::Footer => self.footer_presets.len(),
        }
    }
}
