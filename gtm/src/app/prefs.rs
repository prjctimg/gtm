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

pub(crate) fn left_pane_defaults() -> Vec<String> {
    LIBRARY_CATEGORIES.iter().map(|s| s.to_string()).collect()
}

pub(crate) fn default_show_preview() -> bool {
    true
}

/// Seconds the cursor must rest on a radio or chart row before its preview
/// starts. Zero disables the hover preview.
pub(crate) fn default_preview_delay_secs() -> u64 {
    3
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
    /// Seconds of keyboard, paste or mouse inactivity before the visualizer
    /// takes over the library view. Zero disables daydreaming.
    #[serde(default = "default_daydream_secs")]
    pub(crate) daydream_secs: u64,
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
    #[serde(default = "cover_cache_mb")]
    pub(crate) cover_cache_mb: u64,
    /// Discord application id for Rich Presence, written by the Setup form.
    /// `None` disables presence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) discord_id: Option<String>,
    #[serde(default = "default_fetch_lyrics")]
    pub(crate) auto_fetch_lyrics: bool,
    #[serde(default = "default_icon_style")]
    pub(crate) icon_style: String,
    #[serde(default)]
    pub(crate) hide_footer: bool,
    /// Library category names offered, in display order. The list itself lives
    /// in the Alt+. picker; the key keeps its original name so existing configs
    /// keep working. Unknown names are ignored; an empty list falls back to the
    /// full default set so the picker can never be bricked from TOML. Indices
    /// into `LIBRARY_CATEGORIES` stay stable — this only filters/orders them.
    #[serde(default = "left_pane_defaults")]
    pub(crate) left_pane_lists: Vec<String>,
    /// Master switch for the left-pane track preview card, which is now all the
    /// left pane holds.
    #[serde(default = "default_show_preview")]
    pub(crate) show_preview: bool,
    /// How long the cursor rests on a radio or chart row before its sound
    /// preview starts. Zero disables the preview.
    #[serde(default = "default_preview_delay_secs")]
    pub(crate) preview_delay_secs: u64,
    /// Whether the first-run welcome pane is shown at launch.
    ///
    /// A flag rather than "is there a config file", which is what this used to
    /// be. That inferred the answer from a file that only exists once the user
    /// has changed some *other* setting, so a user who read nothing and changed
    /// nothing was shown the welcome on every single launch, forever, with no
    /// way to say they had seen it. `H` on the pane clears this and writes the
    /// file, which is what finally makes it stick.
    #[serde(default = "default_true")]
    pub(crate) show_welcome: bool,
    /// Whether the daemon resumes playback on startup instead of restoring the
    /// last track paused.
    ///
    /// Off by default. The daemon outlives the client, so starting playback at
    /// its own startup means the machine makes noise at whoever is in the room
    /// whenever they next touch anything -- and "touch anything" includes the
    /// client connecting, which is not something anyone asked for.
    #[serde(default)]
    pub(crate) resume_on_start: bool,
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_cover_provider() -> String {
    "auto".into()
}

pub(crate) fn cover_cache_mb() -> u64 {
    512
}

/// Seconds of inactivity before the visualizer takes over. Long enough to be an
/// absence rather than a pause.
pub(crate) const DEFAULT_DAYDREAM_SECS: u64 = 60;

pub(crate) fn default_daydream_secs() -> u64 {
    DEFAULT_DAYDREAM_SECS
}

/// Keystroke settle time before a provider search fires. Short enough to feel
/// live while still collapsing a fast typist's burst into one request.
pub(crate) const SEARCH_DEBOUNCE_MS: u64 = 250;

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
    // Dark, not auto: a music player is a dark-room app and the reactive wash
    // is tuned against a dark base. Leaving it on auto meant a light-mode
    // desktop got the album colours washed into a light chrome, where the
    // contrast adjustments in `derive_theme` pull the wrong way. A user who
    // wants light sets `light` explicitly.
    "dark".into()
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
            // On by default: the palette is the one thing the daemon computes
            // for every track, and a feature nobody sees is a feature nobody
            // turns on. The intensity is what bounds it, so this is safe to
            // ship enabled.
            reactive_theme: true,
            reactive_theme_intensity: default_reactive_intensity(),
            footer_preset_name: default_preset_name(),
            progress_style: ProgressStyle::default(),
            visualizer_preset: VisualizerPreset::default(),
            daydream_secs: DEFAULT_DAYDREAM_SECS,
            extensions: ExtensionsConfig::default(),
            time_format: default_time_format(),
            theme_mode: default_theme_mode(),
            track_sort: default_track_sort(),
            keybindings: std::collections::HashMap::new(),
            notification_modes: default_notification_modes(),
            footer_key_action: FooterKeyAction::default(),
            cover_provider: default_cover_provider(),
            cover_cache_mb: cover_cache_mb(),
            discord_id: None,
            auto_fetch_lyrics: default_fetch_lyrics(),
            icon_style: default_icon_style(),
            hide_footer: false,
            left_pane_lists: left_pane_defaults(),
            show_preview: true,
            preview_delay_secs: default_preview_delay_secs(),
            show_welcome: true,
            resume_on_start: false,
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
