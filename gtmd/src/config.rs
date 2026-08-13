// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon configuration: CLI args, paths, and defaults
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum AudioBackendKind {
    #[default]
    Rodio,
    #[cfg(feature = "pulseaudio")]
    PulseAudio,
}


#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub socket_pulse_path: PathBuf,
    pub library_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_file: PathBuf,
    pub library_paths: Vec<PathBuf>,
    pub log_file: Option<PathBuf>,
    pub verbose: bool,
    pub test_mode: bool,
    pub audio_backend: AudioBackendKind,
}

#[derive(Parser, Debug)]
#[command(name = "gtmd", about = "GTM background audio daemon")]
pub struct DaemonArgs {
    #[arg(long, help = "Unix socket path")]
    pub socket: Option<String>,

    #[arg(long, help = "Library database path")]
    pub library: Option<String>,

    #[arg(long, help = "Config directory path")]
    pub config: Option<String>,

    #[arg(short, long, help = "Enable verbose logging")]
    pub verbose: bool,

    #[arg(long, help = "Test mode (ephemeral socket, no daemonize)")]
    pub test_mode: bool,

    #[arg(long, help = "Audio backend (rodio, pulseaudio)")]
    pub backend: Option<String>,
}

impl DaemonConfig {
    pub fn load(args: &DaemonArgs) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let home_path = PathBuf::from(&home);

        let data_dir = if let Some(ref c) = args.config {
            PathBuf::from(c)
        } else {
            let base = std::env::var("XDG_DATA_HOME")
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home_path.join(".local/share"));
            base.join("gtm")
        };

        let config_dir = if let Some(ref c) = args.config {
            PathBuf::from(c)
        } else {
            let base = std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home_path.join(".config"));
            base.join("gtm")
        };

        let cache_dir = if let Some(ref c) = args.config {
            PathBuf::from(c).join("cache")
        } else {
            let base = std::env::var("XDG_CACHE_HOME")
                .ok()
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home_path.join(".cache"));
            base.join("gtm")
        };

        let socket_path = if let Some(ref s) = args.socket {
            PathBuf::from(s)
        } else {
            gtm_core::resolve_command_socket()
        };

        let socket_pulse_path = if let Some(ref s) = args.socket {
            let mut p = PathBuf::from(s);
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "gtmd.sock".into());
            p.set_file_name(format!("{name}.pulse"));
            p
        } else {
            gtm_core::resolve_pulse_socket()
        };

        let library_path = if let Some(ref l) = args.library {
            PathBuf::from(l)
        } else {
            data_dir.join("library.db")
        };

        let log_file = if args.test_mode {
            None
        } else {
            Some(data_dir.join("gtmd.log"))
        };

        let audio_backend = match args.backend.as_deref() {
            #[cfg(feature = "pulseaudio")]
            Some("pulseaudio") => AudioBackendKind::PulseAudio,
            Some("rodio") => AudioBackendKind::Rodio,
            // No explicit backend: on Termux, rodio/cpal cannot open an audio
            // device, so default to PulseAudio when it is compiled in.
            #[cfg(feature = "pulseaudio")]
            _ if gtm_core::is_termux() => {
                eprintln!(
                    "gtmd: Termux detected — using the PulseAudio backend. \
                     Ensure it is running: pulseaudio --start --exit-idle-time=-1"
                );
                AudioBackendKind::PulseAudio
            }
            #[cfg(not(feature = "pulseaudio"))]
            _ if gtm_core::is_termux() => {
                eprintln!(
                    "gtmd: Termux detected but this build lacks the `pulseaudio` feature. \
                     Rebuild with `--features pulseaudio` so audio can be output on Termux."
                );
                AudioBackendKind::Rodio
            }
            _ => AudioBackendKind::Rodio,
        };

        // Default library paths: data_dir/audio and user's Music directory
        let mut library_paths = vec![data_dir.join("audio")];
        if let Ok(home) = std::env::var("HOME") {
            let music = PathBuf::from(home).join("Music");
            if music.exists() {
                library_paths.push(music);
            }
        }
        // Termux: also scan shared storage (/sdcard/Music)
        library_paths.extend(gtm_core::termux_music_dirs());

        let state_file = data_dir.join("state.json");

        DaemonConfig {
            socket_path,
            socket_pulse_path,
            library_path,
            config_dir,
            cache_dir,
            data_dir,
            state_file,
            library_paths,
            log_file,
            verbose: args.verbose,
            test_mode: args.test_mode,
            audio_backend,
        }
    }

    pub fn create_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.config_dir)?;
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = self.socket_pulse_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(ref log) = self.log_file {
            if let Some(parent) = log.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(())
    }
}
