// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Config-driven extension registry for the gtm TUI.
//
// Core playback, DSP, library, IPC and the event bus stay in `gtm-core`;
// optional user-facing surfaces (visualizer, floating notification cards,
// the notification settings overlay) live in the TUI and can be switched
// off per-session via the `[extensions]` table in the TUI config:
//
//   [extensions]
//   visualizer = true
//   floating_notifications = true
//   notifications_overlay = true
//
// When an extension is disabled the corresponding IPC stream is zeroed at
// the client (e.g. spectrum is not applied to the live state) and its
// keybindings report the extension as unavailable instead of silently
// doing nothing.

use serde::{Deserialize, Serialize};

/// Stable identifiers for the configurable extensions. Serialized as the
/// snake_case config key of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionId {
    Visualizer,
    FloatingNotifications,
    NotificationOverlay,
}

impl ExtensionId {
    pub const ALL: [ExtensionId; 3] = [
        ExtensionId::Visualizer,
        ExtensionId::FloatingNotifications,
        ExtensionId::NotificationOverlay,
    ];

    /// Config key (also the serde key used in `ExtensionsConfig`).
    pub fn key(self) -> &'static str {
        match self {
            ExtensionId::Visualizer => "visualizer",
            ExtensionId::FloatingNotifications => "floating_notifications",
            ExtensionId::NotificationOverlay => "notifications_overlay",
        }
    }

    /// Human-facing name used in pickers and help text.
    pub fn label(self) -> &'static str {
        match self {
            ExtensionId::Visualizer => "Audio Visualizer",
            ExtensionId::FloatingNotifications => "Floating Notifications",
            ExtensionId::NotificationOverlay => "Notification Overlay",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ExtensionId::Visualizer => "Spectrum analyzer + optional spectra in the footer.",
            ExtensionId::FloatingNotifications => "Floating notification cards for events.",
            ExtensionId::NotificationOverlay => "Full notification history overlay.",
        }
    }

    /// The default for every registered extension is enabled; users opt out.
    pub fn enabled_by_default(self) -> bool {
        true
    }
}

/// The `[extensions]` config table. All fields default to enabled so an
/// absent table (or a pre-registry config file) behaves exactly like the
/// current codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionsConfig {
    pub visualizer: bool,
    pub floating_notifications: bool,
    pub notifications_overlay: bool,
}

impl Default for ExtensionsConfig {
    fn default() -> Self {
        Self {
            visualizer: true,
            floating_notifications: true,
            notifications_overlay: true,
        }
    }
}

impl ExtensionsConfig {
    /// Returns the effective enablement of a registered extension.
    pub fn is_enabled(&self, id: ExtensionId) -> bool {
        match id {
            ExtensionId::Visualizer => self.visualizer,
            ExtensionId::FloatingNotifications => self.floating_notifications,
            ExtensionId::NotificationOverlay => self.notifications_overlay,
        }
    }

    pub fn is_disabled(&self, id: ExtensionId) -> bool {
        !self.is_enabled(id)
    }

    /// Mutate a single extension's enablement (used by pickers).
    pub fn set(&mut self, id: ExtensionId, enabled: bool) {
        match id {
            ExtensionId::Visualizer => self.visualizer = enabled,
            ExtensionId::FloatingNotifications => self.floating_notifications = enabled,
            ExtensionId::NotificationOverlay => self.notifications_overlay = enabled,
        }
    }

    /// Iterate the registry: `(id, enabled)` pairs in declaration order.
    pub fn registry(&self) -> Vec<(ExtensionId, bool)> {
        ExtensionId::ALL
            .iter()
            .map(|&id| (id, self.is_enabled(id)))
            .collect()
    }
}
