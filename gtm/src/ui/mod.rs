// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: layout, widgets, and theme application
//
// This is free software released under the GPL-3.0 license.
// The imports below are this module's single shared import set: every
// submodule reaches them with one `use crate::ui::*;` instead of
// repeating them, so a few are unused here by design.
#![allow(unused_imports)]
use std::borrow::Cow;
use std::path::PathBuf;

use crate::app::fuzzy_match;
use crate::app::{
    App, InputMode, LIBRARY_CATEGORIES, LibraryPick, NotifMode, NotifType, NotificationKind,
    RadioPick, RadioSection, TrackInfoKind, ZenSurface, folder_name, lyrics_are_synced,
    no_image_protocol, setup_selection,
};
use crate::extensions::ExtensionId;
use crate::footer::{
    classify_remote_source, draw as footer_draw, format_duration, format_uptime, is_live_stream,
    render as footer_render,
};
use crate::mouse::MouseZone;
use crate::picker::{Picker, PickerId, PickerSource};
use crate::progress::{ProgressStyle, render_progress, render_progress_styled, render_ratio};
use crate::shared::daemon::ensure_daemon_running;
use crate::shared::global::{EqPreset, PlaybackStatus};
use crate::shared::ipc::HealthStatus;
use crate::shared::log::redirect_stderr;
use crate::shared::radio::RadioStation;
use crate::shared::resolve_command_socket;
use crate::shared::spotify::SpotifySearchKind;
use crate::shared::track::{LrcData, TrackInfo};
use crate::theme::blend_colors;
pub use crate::theme::readable_fg;
use crate::visualizer::VisualizerPreset;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

/// Grouped render helpers: previously free `render_*` functions.
pub struct Render;

pub(crate) struct Pickers;
pub mod chrome;
pub mod command;
pub mod help;
pub mod icons;
pub mod pickers;
pub mod text;
pub mod widgets;

// One glob per helper module: every leaf then needs a single
// `use crate::ui::*;` instead of importing each shared item itself.
pub(crate) use chrome::*;
pub(crate) use icons::*;
pub(crate) use pickers::spotify::spotify_waiting_lines;
pub(crate) use text::*;
pub(crate) use widgets::*;

pub use chrome::{render, run_tui};
pub use command::{COMMAND_GROUPS, Command, CommandPalette};
pub use help::{CROSSFADE_DURATIONS, HELP_LINES};
pub(crate) use icons::{
    cover_provider_label, provider_icon, theme_mode_label, use_nerd_fonts,
};
pub(crate) use text::format_duration_short;
pub(crate) use widgets::{COVER_H, COVER_W, step_viewport};
