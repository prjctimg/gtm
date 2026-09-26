use crate::app::*;

pub(crate) fn prefs_path() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".config")
        });
    let dir = config.join("gtm");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.toml")
}

/// Return the config file path, creating the config directory and a
/// default (pretty-printed) `config.toml` if it does not exist yet. Used by
/// the `gtm config` CLI command to give the editor a valid starting point.
pub(crate) fn ensure_prefs_file() -> std::path::PathBuf {
    let path = prefs_path();
    if !path.exists()
        && let Ok(toml_s) = toml::to_string_pretty(&Prefs::default())
    {
        let _ = std::fs::write(&path, format!("{toml_s}\n"));
    }
    path
}

/// Ordered list of all equalizer presets (order matches the picker).
pub(crate) const EQ_PRESETS: [EqPreset; 16] = [
    EqPreset::Flat,
    EqPreset::Normal,
    EqPreset::Pop,
    EqPreset::Rock,
    EqPreset::Jazz,
    EqPreset::Classical,
    EqPreset::Bass,
    EqPreset::Vocal,
    EqPreset::Electronic,
    EqPreset::HipHop,
    EqPreset::Latin,
    EqPreset::Acoustic,
    EqPreset::Podcast,
    EqPreset::Dance,
    EqPreset::Headphones,
    EqPreset::Speaker,
];

pub(crate) fn default_fetch_lyrics() -> bool {
    true
}

pub(crate) fn default_icon_style() -> String {
    "mdi".to_string()
}

pub(crate) fn default_left_pane_lists() -> Vec<String> {
    LIBRARY_CATEGORIES.iter().map(|s| s.to_string()).collect()
}

pub(crate) fn default_show_preview() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Prefs {
    #[serde(default = "default_theme_name")]
    pub(crate) theme_name: String,
    #[serde(default)]
    pub(crate) transparent_bg: bool,
    #[serde(default)]
    pub(crate) transparent_pickers: bool,
    #[serde(default)]
    pub(crate) reactive_theme: bool,
    #[serde(default = "default_reactive_intensity")]
    pub(crate) reactive_theme_intensity: f32,
    #[serde(default = "default_preset_name")]
    pub(crate) footer_preset_name: String,
    #[serde(default)]
    pub(crate) progress_style: ProgressStyle,
    #[serde(default)]
    pub(crate) visualizer_preset: VisualizerPreset,
    #[serde(default)]
    pub(crate) extensions: ExtensionsConfig,
    #[serde(default = "default_time_format")]
    pub(crate) time_format: String,
    #[serde(default = "default_theme_mode")]
    pub(crate) theme_mode: String,
    #[serde(default = "default_track_sort")]
    pub(crate) track_sort: TrackSort,
    #[serde(default)]
    pub(crate) keybindings: std::collections::HashMap<String, String>,
    #[serde(default = "default_notification_modes")]
    pub(crate) notification_modes: std::collections::HashMap<String, String>,
    /// Whether the footer echoes a command's name or the key that triggered
    /// it. Config-file only; see [`FooterKeyAction`].
    #[serde(default)]
    pub(crate) footer_key_action: FooterKeyAction,
    #[serde(default = "default_cover_provider")]
    pub(crate) cover_provider: String,
    #[serde(default = "default_cover_cache_mb")]
    pub(crate) cover_cache_mb: u64,
    #[serde(default = "default_fetch_lyrics")]
    pub(crate) auto_fetch_lyrics: bool,
    #[serde(default = "default_icon_style")]
    pub(crate) icon_style: String,
    #[serde(default)]
    pub(crate) hide_footer: bool,
    /// Left-pane category names shown, in display order. Unknown names are
    /// ignored; an empty list falls back to the full default set so the pane
    /// can never be bricked from TOML. Indices into `LIBRARY_CATEGORIES`
    /// stay stable — this only filters/orders the render + navigation.
    #[serde(default = "default_left_pane_lists")]
    pub(crate) left_pane_lists: Vec<String>,
    /// Master switch for the left-pane track preview card.
    #[serde(default = "default_show_preview")]
    pub(crate) show_preview: bool,
}

pub(crate) fn default_cover_provider() -> String {
    "auto".into()
}

pub(crate) fn default_cover_cache_mb() -> u64 {
    512
}

/// On-disk cover cache budget in MiB, shared with the daemon.
pub(crate) const COVER_CACHE_STEPS: [u64; 5] = [128, 256, 512, 1024, 2048];

/// Keystroke settle time before a provider search fires. Short enough to feel
/// live while still collapsing a fast typist's burst into one request.
pub(crate) const SEARCH_DEBOUNCE_MS: u64 = 250;

/// How long each About-window visualization preset stays up.
pub const ABOUT_VIZ_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

/// Decorative About-window visualization: a rotating preset over a synthetic
/// waveform. Purely cosmetic, so the tick never touches audio state.
#[derive(Default)]
pub struct AboutViz {
    pub preset: usize,
    pub bars: Vec<f32>,
    pub phase: f32,
    pub next_preset: Option<std::time::Instant>,
}

/// Cheap deterministic waveform sample (sum of sines plus a hashed ripple), so
/// the About window animates identically on every platform without audio.
pub(crate) fn about_sample(t: f32, i: usize) -> f32 {
    let x = i as f32 * 0.22 + t;
    let v = (x * 1.7).sin() * 0.5 + (x * 0.9 + 1.3).sin() * 0.3 + (x * 3.1).sin() * 0.2;
    // Fold to a 0..1 envelope so every preset sees a plausible level.
    ((v + 1.0) * 0.5).clamp(0.02, 1.0)
}

impl AboutViz {
    /// Advance the synthetic waveform and rotate the preset every
    /// [`ABOUT_VIZ_PERIOD`]. Frame-rate independent via the phase accumulator.
    pub(crate) fn tick(&mut self, now: std::time::Instant) {
        let next = *self.next_preset.get_or_insert(now + ABOUT_VIZ_PERIOD);
        if now >= next {
            self.preset = self.preset.wrapping_add(1);
            self.phase = 0.0;
            self.next_preset = Some(now + ABOUT_VIZ_PERIOD);
            return;
        }
        // ~12 fps is plenty for a decorative loop and keeps this cheap.
        let last = self.phase;
        self.phase = (self.phase + 0.2) % 1000.0;
        if self.phase < last {
            return;
        }
        let n = self.bars.len().max(1);
        for i in 0..n {
            self.bars[i] = about_sample(self.phase * 0.1, i);
        }
    }
}

pub(crate) fn default_theme_name() -> String {
    "Chadrula".into()
}

/// Default reactive background wash strength (matches the pre-0.2.84
/// hardcoded blend factor in `reactive::derive_theme`).
pub(crate) fn default_reactive_intensity() -> f32 {
    0.34
}

pub(crate) fn default_track_sort() -> TrackSort {
    TrackSort::Recents
}

pub(crate) fn default_time_format() -> String {
    "%H:%M".into()
}

pub(crate) fn default_theme_mode() -> String {
    "auto".into()
}

/// Resolve which theme to start with. Honors the user's persisted
/// `theme_name`, unless `theme_mode` is "auto" (the default) and the OS can be
/// queried for its dark/light preference — in which case a theme matching the
/// OS preference is chosen over the saved name. A saved name still wins when
/// it already agrees with the OS preference.
pub(crate) fn resolve_theme_index(themes: &[ThemeEntry], theme_name: &str, mode: &str) -> usize {
    if themes.is_empty() {
        return 0;
    }
    // Only override with the OS preference when in auto mode.
    let os = if mode == "auto" {
        detect_os_theme()
    } else {
        match mode {
            "dark" => Some(ThemeMode::Dark),
            "light" => Some(ThemeMode::Light),
            _ => None,
        }
    };
    if let Some(os) = os {
        let os_light = os == ThemeMode::Light;
        // Prefer an exact match on the persisted name if it agrees with the OS.
        if let Some(idx) = themes.iter().position(|t| t.name == theme_name)
            && themes[idx].light == os_light
        {
            return idx;
        }
        // Otherwise pick the first theme whose light flag matches the OS.
        if let Some(idx) = themes.iter().position(|t| t.light == os_light) {
            return idx;
        }
    }
    themes
        .iter()
        .position(|t| t.name == theme_name)
        .unwrap_or(0)
}

pub(crate) fn default_preset_name() -> String {
    "Default".into()
}

/// Default per-type notification modes: everything Footer. The floating
/// surface is reserved for the Up Next card, so out-of-the-box nothing else
/// floats; users who want a category to float opt in from the settings.
pub(crate) fn default_notification_modes() -> std::collections::HashMap<String, String> {
    NotifType::ALL
        .iter()
        .map(|t| {
            (
                t.as_str().to_string(),
                NotifMode::Footer.as_str().to_string(),
            )
        })
        .collect()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme_name: default_theme_name(),
            transparent_bg: false,
            transparent_pickers: false,
            reactive_theme: false,
            reactive_theme_intensity: default_reactive_intensity(),
            footer_preset_name: default_preset_name(),
            progress_style: ProgressStyle::default(),
            visualizer_preset: VisualizerPreset::default(),
            extensions: ExtensionsConfig::default(),
            time_format: default_time_format(),
            theme_mode: default_theme_mode(),
            track_sort: default_track_sort(),
            keybindings: std::collections::HashMap::new(),
            notification_modes: default_notification_modes(),
            footer_key_action: FooterKeyAction::default(),
            cover_provider: default_cover_provider(),
            cover_cache_mb: default_cover_cache_mb(),
            auto_fetch_lyrics: default_fetch_lyrics(),
            icon_style: default_icon_style(),
            hide_footer: false,
            left_pane_lists: default_left_pane_lists(),
            show_preview: true,
        }
    }
}

pub(crate) fn load_prefs() -> Prefs {
    let path = prefs_path();
    let Ok(s) = std::fs::read_to_string(&path) else {
        return Prefs::default();
    };
    toml::from_str::<Prefs>(&s).unwrap_or_default()
}

pub(crate) fn save_prefs(prefs: &Prefs) {
    if let Ok(s) = toml::to_string(prefs) {
        let _ = std::fs::write(prefs_path(), s);
    }
}
