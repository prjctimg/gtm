// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: layout, widgets, and theme application
//
// This is free software released under the GPL-3.0 license.
// The imports below are this module's single shared import set: every
// submodule reaches them with one `use crate::ui::*;` instead of
// repeating them, so a few are unused here by design.
#![allow(unused_imports)]
pub(crate) use std::borrow::Cow;
pub(crate) use std::path::PathBuf;

pub(crate) use crate::app::fuzzy_match;
pub(crate) use crate::app::{
    App, InputMode, LIBRARY_CATEGORIES, LibraryPick, NotifMode, NotifType, NotificationKind,
    RadioPick, RadioSection, TrackInfoKind, ZenSurface, folder_name, lyrics_are_synced,
    no_image_protocol, setup_selection,
};
pub(crate) use crate::footer::{
    classify_remote_source, draw as footer_draw, format_duration, format_uptime, is_live_stream,
    render as footer_render,
};
pub(crate) use crate::mouse::MouseZone;
pub(crate) use crate::picker::{Picker, PickerId, PickerSource};
pub(crate) use crate::progress::{
    ProgressStyle, render_progress, render_progress_styled, render_ratio,
};
pub(crate) use crate::shared::daemon::ensure_daemon_running;
pub(crate) use crate::shared::global::{EqPreset, PlaybackStatus};
pub(crate) use crate::shared::ipc::HealthStatus;
pub(crate) use crate::shared::log::redirect_stderr;
pub(crate) use crate::shared::radio::RadioStation;
pub(crate) use crate::shared::resolve_command_socket;
pub(crate) use crate::shared::spotify::{SpotifySearchKind, pretty_id};
pub(crate) use crate::shared::track::{LrcData, TrackInfo};
pub(crate) use crate::theme::blend_colors;
pub use crate::theme::readable_fg;
pub(crate) use crate::ui::pickers::settings::pre_gain_label;
pub(crate) use crate::visualizer::VisualizerPreset;
pub(crate) use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
pub(crate) use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
pub(crate) use ratatui::Terminal;
pub(crate) use ratatui::backend::CrosstermBackend;
pub(crate) use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect, Size};
pub(crate) use ratatui::style::Color;
pub(crate) use ratatui::style::{Modifier, Style};
pub(crate) use ratatui::text::{Line, Span};
pub(crate) use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap,
};
pub(crate) use ratatui_image::protocol::StatefulProtocol;
pub(crate) use ratatui_image::{Resize, StatefulImage};

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
pub(crate) use icons::{cover_provider_label, provider_icon, theme_mode_label, use_nerd_fonts};
pub(crate) use text::format_duration_short;
pub(crate) use widgets::{COVER_H, COVER_W, step_viewport};
