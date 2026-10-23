use crate::app::*;
use crate::providers::radio::empty_station;

impl App {
    pub fn search_radio(&mut self, query: String) {
        self.radio.search.clear();
        self.radio.search_pending = true;
        self.radio.section = RadioSection::Results;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.radio().search(&query, 50).await {
                Ok(s) => {
                    let _ = ipc_tx.send(IpcResult::RadioSearch(s));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio search failed: {e}"));
                }
            }
        });
    }

    /// Match a directory station against the picker query on the active
    /// filter field. Rating is a numeric votes threshold; the other fields are
    /// case-insensitive substring matches.
    pub(crate) fn station_matches_filter(
        station: &RadioStation,
        q: &str,
        filter: RadioFilter,
    ) -> bool {
        match filter {
            RadioFilter::Name => station.name.to_lowercase().contains(q),
            RadioFilter::Tags => station.tags.to_lowercase().contains(q),
            RadioFilter::Country => station.country.to_lowercase().contains(q),
            RadioFilter::Rating => q.parse::<u64>().is_ok_and(|min| station.votes >= min),
        }
    }

    /// Build the selectable row list for the unified Radio picker from the
    /// active section and query filter. Mirrors the `LibraryPick` rows of the
    /// SearchLibrary picker so selection indexes map 1:1 into the rendered
    /// list. The Root view merges Saved / Top / Tags / Countries with section
    /// headers (only non-empty sections at a given filter).
    pub fn radio_picks(&self) -> Vec<RadioPick> {
        let q = self
            .pickers
            .top()
            .map(|o| o.query.trim().to_lowercase())
            .unwrap_or_default();
        let filter = self.radio.filter;
        let matches =
            |s: &RadioStation| q.is_empty() || Self::station_matches_filter(s, &q, filter);
        match self.radio.section {
            RadioSection::Stations => self
                .radio
                .browse_stations
                .iter()
                .enumerate()
                .filter(|(_, s)| matches(s))
                .map(|(i, _)| RadioPick::Station(i))
                .collect(),
            RadioSection::Results => self
                .radio
                .search
                .iter()
                .enumerate()
                .filter(|(_, s)| matches(s))
                .map(|(i, _)| RadioPick::Station(i))
                .collect(),
            RadioSection::Root => {
                let mut rows: Vec<RadioPick> = Vec::new();
                let custom_match = |s: &CustomRadioStation| {
                    q.is_empty()
                        || (filter == RadioFilter::Name
                            && s.name.to_lowercase().contains(q.as_str()))
                };
                if self.radio.custom.iter().any(custom_match) {
                    rows.push(RadioPick::Header("Saved"));
                    rows.extend(
                        self.radio
                            .custom
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| custom_match(s))
                            .map(|(i, _)| RadioPick::Custom(i)),
                    );
                }
                if self.radio.top.iter().any(&matches) {
                    rows.push(RadioPick::Header("Top"));
                    rows.extend(
                        self.radio
                            .top
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| matches(s))
                            .map(|(i, _)| RadioPick::Station(i)),
                    );
                }
                let tag_match = |t: &RadioTag| {
                    q.is_empty()
                        || (matches!(filter, RadioFilter::Name | RadioFilter::Tags)
                            && t.name.to_lowercase().contains(q.as_str()))
                };
                if self.radio.browse_tags.iter().any(tag_match) {
                    rows.push(RadioPick::Header("Tags"));
                    rows.extend(
                        self.radio
                            .browse_tags
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| tag_match(t))
                            .map(|(i, _)| RadioPick::Tag(i)),
                    );
                }
                let country_match = |c: &RadioCountry| {
                    q.is_empty()
                        || (matches!(filter, RadioFilter::Name | RadioFilter::Country)
                            && c.name.to_lowercase().contains(q.as_str()))
                };
                if self.radio.browse_countries.iter().any(country_match) {
                    rows.push(RadioPick::Header("Countries"));
                    rows.extend(
                        self.radio
                            .browse_countries
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| country_match(c))
                            .map(|(i, _)| RadioPick::Country(i)),
                    );
                }
                rows
            }
        }
    }

    /// Seed the unified Radio picker's remote lists (Top, Tags, Countries)
    /// independently in the background, so a partial failure still populates
    /// the rest. Skips lists that already loaded or are in flight unless
    /// `force` is set (the `r` refresh key).
    pub fn seed_radio_picker(&mut self, force: bool) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        if (force || self.radio.top.is_empty()) && !self.radio.top_pending {
            self.radio.top_pending = true;
            let c2 = c.clone();
            let tx2 = ipc_tx.clone();
            tokio::spawn(async move {
                match c2.radio().top(50).await {
                    Ok(s) => {
                        let _ = tx2.send(IpcResult::RadioTop(s));
                    }
                    Err(e) => {
                        self_err(&tx2, format!("radio top failed: {e}"));
                    }
                }
            });
        }
        let lists_missing =
            self.radio.browse_tags.is_empty() || self.radio.browse_countries.is_empty();
        if (force || lists_missing) && !self.radio.browse_pending {
            self.radio.browse_pending = true;
            let tags_c = c.clone();
            let tags_tx = ipc_tx.clone();
            tokio::spawn(async move {
                match tags_c.radio().tags(200).await {
                    Ok(t) => {
                        let _ = tags_tx.send(IpcResult::RadioTags(t));
                    }
                    Err(e) => {
                        self_err(&tags_tx, format!("radio tags failed: {e}"));
                    }
                }
            });
            let countries_c = c.clone();
            let countries_tx = ipc_tx.clone();
            tokio::spawn(async move {
                match countries_c.radio().countries(200).await {
                    Ok(cs) => {
                        let _ = countries_tx.send(IpcResult::RadioCountries(cs));
                    }
                    Err(e) => {
                        self_err(&countries_tx, format!("radio countries failed: {e}"));
                    }
                }
            });
        }
    }

    /// Re-fetch whatever list the current picker section shows (the `r` key).
    pub fn refresh_radio_section(&mut self) {
        match self.radio.section {
            RadioSection::Root => {
                self.refresh_custom_stations();
                self.seed_radio_picker(true);
            }
            RadioSection::Stations => self.fetch_browse_stations(),
            RadioSection::Results => {
                let q = self
                    .pickers
                    .top()
                    .map_or(String::new(), |o| o.query.clone());
                if !q.is_empty() {
                    self.search_radio(q);
                }
            }
        }
    }

    /// (Re)load the stations for the tag/country selected from the Root view.
    pub fn fetch_browse_stations(&mut self) {
        self.radio.browse_stations.clear();
        self.radio.browse_stations_pending = true;
        self.radio.section = RadioSection::Stations;
        let (c, ipc_tx) = (self.client.clone(), self.ipc_tx.clone());
        let topic = self.radio.browse_topic.clone();
        let by = self.radio.browse_by;
        tokio::spawn(async move {
            let r = match by {
                RadioBrowseBy::Tag => c
                    .radio()
                    .stations_by_tag(&topic, 50)
                    .await
                    .map(IpcResult::RadioBrowseStations),
                RadioBrowseBy::Country => c
                    .radio()
                    .stations_by_country(&topic, 50)
                    .await
                    .map(IpcResult::RadioBrowseStations),
            };
            match r {
                Ok(ipc) => {
                    let _ = ipc_tx.send(ipc);
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio stations failed: {e}"));
                }
            }
        });
    }

    /// (Re)read the custom stations list from `radios.toml` into the left
    /// pane Radio category. Read-on-demand so external edits are picked up.
    pub fn refresh_custom_stations(&mut self) {
        if let Ok(stations) = crate::shared::custom::list_custom_stations() {
            self.radio.custom = stations;
        }
    }

    /// Save a Radio Browser station as a custom station, keyed by its browser
    /// uuid so the daemon can re-resolve a fresh stream URL and fetch its
    /// favicon. No-ops when an identical station already exists.
    pub fn save_custom_station(&mut self, station: &RadioStation) {
        if self.radio.custom.iter().any(|s| {
            s.uuid.as_deref() == Some(&station.id) || s.name.eq_ignore_ascii_case(&station.name)
        }) {
            // Radio save confirmations are fully silent (history only).
            self.notify_silent(
                "Radio",
                format!("Already saved \"{}\"", station.name),
                NotificationKind::Info,
            );
            return;
        }
        match crate::shared::custom::add_custom_station(
            &station.name,
            &station.url_resolved,
            Some(&station.id),
        ) {
            Ok(_) => {
                self.refresh_custom_stations();
                // Radio save confirmations are fully silent (history only).
                self.notify_silent(
                    "Radio",
                    format!("Saved \"{}\" to custom stations", station.name),
                    NotificationKind::Success,
                );
            }
            Err(e) => {
                self.notify_titled(
                    "Radio",
                    format!("Save failed: {e}"),
                    NotificationKind::Error,
                    true,
                    NotifType::Prefs,
                );
            }
        }
    }

    /// Reset the left-pane Radio category to its default view.
    ///
    /// The left pane used to list only `radios.toml`, so entering the category
    /// looked like an empty directory until the user found the picker. This
    /// loads the directory's Top list alongside the saved stations and starts
    /// every visit on the merged `All` scope.
    pub fn prepare_radio_left(&mut self) {
        self.refresh_custom_stations();
        self.radio.left_scope = RadioLeftScope::All;
        self.radio.left_stations.clear();
        self.radio.left_tag_pending = false;
        self.radio.left_tag_loaded = None;
        self.seed_radio_picker(false);
    }

    /// The flat row list behind the left-pane Radio category.
    ///
    /// The picker's tree is deliberately not reused here: a header row would
    /// make Enter land on a label, and the category has one scope at a time.
    /// `All` merges saved stations with the directory's Top list, skipping
    /// directory rows already saved so the same station does not appear twice.
    pub fn radio_left_rows(&self) -> Vec<RadioLeftRow> {
        match &self.radio.left_scope {
            RadioLeftScope::All => {
                let mut rows: Vec<RadioLeftRow> = self
                    .radio
                    .custom
                    .iter()
                    .enumerate()
                    .map(|(i, _)| RadioLeftRow::Custom(i))
                    .collect();
                for (i, station) in self.radio.top.iter().enumerate() {
                    let already_saved = self.radio.custom.iter().any(|saved| {
                        saved.uuid.as_deref() == Some(station.id.as_str())
                            || (saved.name.eq_ignore_ascii_case(&station.name)
                                && saved.url == station.playable_url())
                    });
                    if !already_saved {
                        rows.push(RadioLeftRow::Top(i));
                    }
                }
                rows
            }
            RadioLeftScope::Saved => (0..self.radio.custom.len())
                .map(RadioLeftRow::Custom)
                .collect(),
            RadioLeftScope::Tag(_) => (0..self.radio.left_stations.len())
                .map(RadioLeftRow::Tag)
                .collect(),
        }
    }

    /// The name of one left-pane Radio row, for the list renderer.
    pub fn radio_left_row_name(&self, row: RadioLeftRow) -> Option<&str> {
        match row {
            RadioLeftRow::Custom(i) => self.radio.custom.get(i).map(|s| s.name.as_str()),
            RadioLeftRow::Top(i) => self.radio.top.get(i).map(|s| s.name.as_str()),
            RadioLeftRow::Tag(i) => self.radio.left_stations.get(i).map(|s| s.name.as_str()),
        }
    }

    /// The station under the left-pane Radio cursor.
    ///
    /// Saved rows are materialised as `RadioStation` so the info card and the
    /// play path can share one shape with directory rows. The extra fields are
    /// what the card needs but `RadioStation` does not carry: whether this came
    /// from the directory and a saved station's tracklist endpoint.
    pub fn radio_left_selection_at(&self, pos: usize) -> Option<RadioLeftSelection> {
        match *self.radio_left_rows().get(pos)? {
            RadioLeftRow::Custom(i) => {
                let saved = self.radio.custom.get(i)?;
                let mut station = empty_station();
                station.id = saved
                    .uuid
                    .clone()
                    .unwrap_or_else(|| format!("custom:{}", i + 1));
                station.name = saved.name.clone();
                station.url = saved.url.clone();
                station.url_resolved = saved.url.clone();
                Some(RadioLeftSelection {
                    station,
                    directory: saved.uuid.is_some(),
                    tracklist: saved.tracklist.clone(),
                })
            }
            RadioLeftRow::Top(i) => Some(RadioLeftSelection {
                station: self.radio.top.get(i)?.clone(),
                directory: true,
                tracklist: None,
            }),
            RadioLeftRow::Tag(i) => Some(RadioLeftSelection {
                station: self.radio.left_stations.get(i)?.clone(),
                directory: true,
                tracklist: None,
            }),
        }
    }

    /// The station under the cursor, without the card-only fields.
    pub fn radio_left_station_at(&self, pos: usize) -> Option<RadioStation> {
        self.radio_left_selection_at(pos).map(|s| s.station)
    }

    /// The saved station under the cursor, if the row is one.
    ///
    /// Removal is only meaningful for `radios.toml`; a directory row cannot be
    /// deleted from the browser, and the old path assumed every row was saved.
    pub fn radio_left_saved_at(&self, pos: usize) -> Option<&CustomRadioStation> {
        match *self.radio_left_rows().get(pos)? {
            RadioLeftRow::Custom(i) => self.radio.custom.get(i),
            RadioLeftRow::Top(_) | RadioLeftRow::Tag(_) => None,
        }
    }

    /// The scopes `[` / `]` step through: All, Saved, then each directory tag.
    fn radio_left_scopes(&self) -> Vec<RadioLeftScope> {
        let mut scopes = vec![RadioLeftScope::All, RadioLeftScope::Saved];
        scopes.extend(
            self.radio
                .browse_tags
                .iter()
                .map(|tag| RadioLeftScope::Tag(tag.name.clone())),
        );
        scopes
    }

    /// Step the left-pane Radio scope, loading a tag's stations on first use.
    pub(crate) fn cycle_radio_left_scope(&mut self, back: bool) {
        let scopes = self.radio_left_scopes();
        let current = self.radio.left_scope.clone();
        let index = scopes.iter().position(|s| *s == current).unwrap_or(0);
        let len = scopes.len();
        let next = scopes[if back {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        }]
        .clone();

        self.radio.left_scope = next.clone();
        self.radio.left_stations.clear();
        self.radio.left_tag_pending = false;
        self.radio.left_tag_loaded = None;
        self.set_list_pos(0);
        if matches!(next, RadioLeftScope::Tag(_)) {
            self.fetch_left_tag_stations();
        }
        self.update_track_popup();
        self.data_dirty = true;
    }

    /// Load the stations for the tag the left pane is showing.
    pub fn fetch_left_tag_stations(&mut self) {
        let RadioLeftScope::Tag(tag) = self.radio.left_scope.clone() else {
            return;
        };
        self.radio.left_tag_pending = true;
        self.radio.left_tag_loaded = None;
        self.radio.left_stations.clear();
        let (c, ipc_tx) = (self.client.clone(), self.ipc_tx.clone());
        let request_tag = tag.clone();
        tokio::spawn(async move {
            match c.radio().stations_by_tag(&request_tag, 50).await {
                Ok(stations) => {
                    let _ = ipc_tx.send(IpcResult::RadioLeftStations(tag, stations));
                }
                Err(e) => {
                    self_err(&ipc_tx, format!("radio tag stations failed: {e}"));
                    // An answer, even an empty one, releases the "loading" line
                    // instead of leaving the scope spinning forever.
                    let _ = ipc_tx.send(IpcResult::RadioLeftStations(tag, Vec::new()));
                }
            }
        });
    }

    /// The station the radio picker's row `i` names, as the details the preview
    /// strip shows.
    ///
    /// `None` for a header or a drill-down row. A saved station carries only a
    /// name and a URL — the directory publishes the rest, but a station the user
    /// added by hand never went through it — so its details are genuinely empty
    /// and the strip shows what there is rather than blank labelled rows.
    pub fn radio_station_at(&self, i: usize) -> Option<RadioStation> {
        match self.radio_picks().get(i)? {
            RadioPick::Custom(idx) => {
                let saved = self.radio.custom.get(*idx)?;
                Some(RadioStation {
                    id: saved
                        .uuid
                        .clone()
                        .unwrap_or_else(|| format!("custom:{}", idx + 1)),
                    name: saved.name.clone(),
                    url: saved.url.clone(),
                    ..empty_station()
                })
            }
            RadioPick::Station(idx) => {
                let stations: &[RadioStation] = match self.radio.section {
                    RadioSection::Stations => &self.radio.browse_stations,
                    RadioSection::Results => &self.radio.search,
                    RadioSection::Root => &self.radio.top,
                };
                stations.get(*idx).cloned()
            }
            _ => None,
        }
    }

    /// The `radio://` station id the current track is playing, if it is one.
    fn live_station(&self) -> Option<&str> {
        self.state
            .current_track
            .as_ref()?
            .path
            .strip_prefix("radio://")
            .map(|rest| rest.split('/').next().unwrap_or(rest))
    }

    /// Pull the playing station's tracklist on demand. The daemon also mirrors
    /// it into `state` on a refresh tick, so this only has an effect when the
    /// mirror has not landed yet — it fills that gap rather than replacing it,
    /// and the daemon's next refresh overwrites the result either way.
    pub(crate) fn fetch_live_list(&mut self) {
        if !self.state.radio_tracks.tracks.is_empty() {
            return;
        }
        let Some(station) = self.live_station().map(str::to_string) else {
            return;
        };
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match client.radio().tracklist(&station).await {
                Ok(list) if !list.tracks.is_empty() => {
                    let _ = ipc_tx.send(IpcResult::RadioTracklist(list));
                }
                Ok(_) => {}
                Err(e) => {
                    self_err(&ipc_tx, format!("radio tracklist: {e}"));
                }
            }
        });
    }

    /// Enter on the unified Radio picker: activate the highlighted row —    /// play a station, drill into a tag/country, or run a directory search
    /// when the current filter matches nothing.
    pub(crate) fn radio_enter(&mut self) {
        let picks = self.radio_picks();
        let sel = self.pickers.top().map_or(0, |o| o.selected);
        match picks.get(sel).cloned() {
            Some(RadioPick::Custom(i)) => {
                if let Some(station) = self.radio.custom.get(i).cloned() {
                    let id = match station.uuid.as_deref() {
                        Some(uuid) => uuid.to_string(),
                        None => format!("custom:{}", i + 1),
                    };
                    let c = self.client.clone();
                    self.pickers.close_top();
                    tokio::spawn(async move {
                        let _ = c.radio().play(&id, &station.name).await;
                    });
                }
            }
            Some(RadioPick::Station(i)) => {
                let stations: &[RadioStation] = match self.radio.section {
                    RadioSection::Stations => &self.radio.browse_stations,
                    RadioSection::Results => &self.radio.search,
                    RadioSection::Root => &self.radio.top,
                };
                if let Some(station) = stations.get(i).cloned() {
                    let c = self.client.clone();
                    self.pickers.close_top();
                    tokio::spawn(async move {
                        let _ = c.radio().play(&station.id, &station.name).await;
                    });
                }
            }
            Some(RadioPick::Tag(i)) => {
                if let Some(tag) = self.radio.browse_tags.get(i).cloned() {
                    self.radio.browse_by = RadioBrowseBy::Tag;
                    self.radio.browse_topic = tag.name;
                    self.fetch_browse_stations();
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = 0;
                        top.viewport_offset = 0;
                    }
                }
            }
            Some(RadioPick::Country(i)) => {
                if let Some(country) = self.radio.browse_countries.get(i).cloned() {
                    self.radio.browse_by = RadioBrowseBy::Country;
                    self.radio.browse_topic = country.name;
                    self.fetch_browse_stations();
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = 0;
                        top.viewport_offset = 0;
                    }
                }
            }
            Some(RadioPick::Header(_)) => {}
            None => {
                // No row to activate: the filtered view is empty (or only a
                // header showed), so fall back to a directory search.
                let q = self
                    .pickers
                    .top()
                    .map_or(String::new(), |o| o.query.trim().to_string());
                if !q.is_empty() && !self.radio.search_pending {
                    self.search_radio(q);
                }
            }
        }
    }

    /// Move the unified Radio picker's selection by one (wrapping), skipping
    /// the non-actionable section header rows in the merged Root view.
    pub(crate) fn move_radio_selection(&mut self, down: bool) {
        let picks = self.radio_picks();
        if picks.is_empty() {
            return;
        }
        let Some(top) = self.pickers.top_mut() else {
            return;
        };
        let step = if down { 1i64 } else { -1i64 };
        let mut idx = top.selected as i64;
        for _ in 0..picks.len() {
            idx = (idx + step).rem_euclid(picks.len() as i64);
            if !matches!(picks[idx as usize], RadioPick::Header(_)) {
                top.selected = idx as usize;
                return;
            }
        }
    }
}
