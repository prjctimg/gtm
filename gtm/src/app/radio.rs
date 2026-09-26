use crate::app::*;

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
    pub(crate) fn station_matches_filter(station: &RadioStation, q: &str, filter: RadioFilter) -> bool {
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

    /// Enter on the unified Radio picker: activate the highlighted row —
    /// play a station, drill into a tag/country, or run a directory search
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
