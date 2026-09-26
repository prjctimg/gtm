use crate::app::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Category of a notification toast. Each category has an independent
/// visibility mode (`NotifMode`), letting the user keep e.g. playback STATUS
/// toasts quiet while errors stay on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NotifType {
    Playback,
    Prefs,
    NowPlaying,
    Library,
    Downloads,
    Spotify,
    Podcast,
    Radio,
    Lastfm,
    System,
}

impl NotifType {
    pub const ALL: [NotifType; 10] = [
        NotifType::Playback,
        NotifType::Prefs,
        NotifType::NowPlaying,
        NotifType::Library,
        NotifType::Downloads,
        NotifType::Spotify,
        NotifType::Podcast,
        NotifType::Radio,
        NotifType::Lastfm,
        NotifType::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NotifType::Playback => "Playback",
            NotifType::Prefs => "Prefs / UI",
            NotifType::NowPlaying => "Now Playing / Queue",
            NotifType::Library => "Library",
            NotifType::Downloads => "YouTube / Downloads",
            NotifType::Spotify => "Spotify",
            NotifType::Podcast => "Podcast",
            NotifType::Radio => "Radio",
            NotifType::Lastfm => "Last.fm",
            NotifType::System => "System / Errors",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NotifType::Playback => "playback",
            NotifType::Prefs => "prefs",
            NotifType::NowPlaying => "nowplaying",
            NotifType::Library => "library",
            NotifType::Downloads => "downloads",
            NotifType::Spotify => "spotify",
            NotifType::Podcast => "podcast",
            NotifType::Radio => "radio",
            NotifType::Lastfm => "lastfm",
            NotifType::System => "system",
        }
    }

    pub fn from_str_lossy(s: &str) -> NotifType {
        match s {
            "playback" => NotifType::Playback,
            "prefs" => NotifType::Prefs,
            "nowplaying" => NotifType::NowPlaying,
            "library" => NotifType::Library,
            "downloads" => NotifType::Downloads,
            "spotify" => NotifType::Spotify,
            "podcast" => NotifType::Podcast,
            "radio" => NotifType::Radio,
            "lastfm" => NotifType::Lastfm,
            // Unknown/legacy names (e.g. retired provider categories) fall
            // back to the always-visible System bucket.
            _ => NotifType::System,
        }
    }
}

/// How a notification category is surfaced: floating toast, terse footer
/// one-liner, or suppressed (history only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotifMode {
    Floating,
    Footer,
    Off,
}

impl NotifMode {
    pub const ALL: [NotifMode; 3] = [NotifMode::Floating, NotifMode::Footer, NotifMode::Off];

    pub fn label(self) -> &'static str {
        match self {
            NotifMode::Floating => "Floating",
            NotifMode::Footer => "Footer",
            NotifMode::Off => "Off",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NotifMode::Floating => "floating",
            NotifMode::Footer => "footer",
            NotifMode::Off => "off",
        }
    }

    pub fn from_str_lossy(s: &str) -> NotifMode {
        match s {
            "footer" => NotifMode::Footer,
            "off" => NotifMode::Off,
            _ => NotifMode::Floating,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideDirection {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub message: String,
    pub kind: NotificationKind,
    pub expires_at: std::time::Instant,
    pub slide_direction: SlideDirection,
    pub is_volume: bool,
    pub volume_value: u8,
    pub animation_progress: f32,
    pub trivial: bool,
}

#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub title: String,
    pub message: String,
    pub kind: NotificationKind,
    pub at: std::time::Instant,
}

impl App {
    /// System notice that is only recorded in history (and the log file for
    /// errors): it never pops a floating card nor writes a footer line. Used
    /// for petty confirmations (extension-disabled, theme/sort changes,
    /// favourite toggles, cache clears, radio saves) that only need an audit
    /// trail.
    pub fn notify(&mut self, message: impl Into<String>, kind: NotificationKind) {
        let title = match kind {
            NotificationKind::Info | NotificationKind::Warning => "System",
            NotificationKind::Success => "Success",
            NotificationKind::Error => "Error",
        };
        self.notify_silent(title, message, kind);
    }

    /// Record a notice in history only — never a floating card, never a
    /// footer line — regardless of the category's configured mode. Errors are
    /// still persisted to the log file so failures stay reviewable.
    pub fn notify_silent(
        &mut self,
        title: &str,
        message: impl Into<String>,
        kind: NotificationKind,
    ) {
        let message = message.into();
        if kind == NotificationKind::Error {
            log(&format!("{title}: {message}"));
        }
        self.notification_history.insert(
            0,
            NotificationRecord {
                title: title.to_string(),
                message: message.clone(),
                kind: kind.clone(),
                at: std::time::Instant::now(),
            },
        );
        self.notification_history.truncate(50);
    }

    pub fn notify_titled(
        &mut self,
        title: &str,
        message: impl Into<String>,
        kind: NotificationKind,
        trivial: bool,
        ntype: NotifType,
    ) {
        self.notify_typed(title, message, kind, trivial, ntype);
    }

    /// Core emitter: record history and surface `message` per the category's
    /// configured mode. History is always kept so nothing is lost when a
    /// category is muted or demoted to the footer.
    pub fn notify_typed(
        &mut self,
        title: &str,
        message: impl Into<String>,
        kind: NotificationKind,
        trivial: bool,
        ntype: NotifType,
    ) {
        let message = message.into();
        // Every error is persisted to the log file regardless of whether its
        // category surfaces a floating card, so failures stay reviewable.
        if kind == NotificationKind::Error {
            log(&format!("{title}: {message}"));
        }
        self.notification_history.insert(
            0,
            NotificationRecord {
                title: title.to_string(),
                message: message.clone(),
                kind: kind.clone(),
                at: std::time::Instant::now(),
            },
        );
        self.notification_history.truncate(50);
        let mode = self
            .notification_modes
            .get(&ntype)
            .copied()
            .unwrap_or(NotifMode::Floating);
        match mode {
            NotifMode::Floating
                if self
                    .extensions
                    .is_disabled(ExtensionId::FloatingNotifications) =>
            {
                // The floating-card surface is a configurable extension; when
                // disabled the event is demoted to the footer (or dropped by
                // the next arm) rather than lost, and the per-category mode is
                // left untouched.
                self.notify_footer(format!("{title}: {message}"));
            }
            NotifMode::Floating => {
                let expires_at = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                self.notifications.push(Notification {
                    title: title.to_string(),
                    message,
                    kind,
                    expires_at,
                    slide_direction: SlideDirection::Right,
                    is_volume: false,
                    volume_value: 0,
                    animation_progress: 0.0,
                    trivial,
                });
            }
            NotifMode::Footer => {
                self.notify_footer(format!("{title}: {message}"));
            }
            NotifMode::Off => {
                // History only.
            }
        }
    }

    pub fn notify_footer(&mut self, message: impl Into<String>) {
        self.footer_notification = Some((
            message.into(),
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        ));
    }

    pub fn notify_volume(&mut self, volume: u8) {
        // Volume is a Playback-class notification: honor its configured mode.
        let mode = self
            .notification_modes
            .get(&NotifType::Playback)
            .copied()
            .unwrap_or(NotifMode::Floating);
        if mode == NotifMode::Off {
            return;
        }
        let now = std::time::Instant::now();
        if mode == NotifMode::Footer {
            let icon = if use_nerd_fonts() {
                "\u{f057e} " // nf-md-volume-high
            } else {
                "\u{1f50a} " // 🔊
            };
            self.notify_footer(format!("{icon}{volume}%"));
            return;
        }
        if let Some(existing) = self
            .notifications
            .iter_mut()
            .find(|n| n.is_volume && n.expires_at > now)
        {
            existing.volume_value = volume;
            existing.expires_at = now + std::time::Duration::from_millis(1500);
            return;
        }
        let expires_at = now + std::time::Duration::from_millis(1500);
        self.notifications.push(Notification {
            title: "Volume".to_string(),
            message: format!("Vol {}", volume),
            kind: NotificationKind::Info,
            expires_at,
            slide_direction: SlideDirection::Right,
            is_volume: true,
            volume_value: volume,
            animation_progress: 0.0,
            trivial: true,
        });
    }
}
