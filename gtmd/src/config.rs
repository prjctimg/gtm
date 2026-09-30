// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon configuration: CLI args, paths, and defaults
//
// This is free software released under the GPL-3.0 license.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

use crate::cover::CoverProvider;
use gtm::shared::{is_termux, resolve_command_socket, resolve_pulse_socket, termux_music_dirs};
use serde::{Deserialize, Serialize};

/// Default loopback port for the status endpoint.
pub const WEB_PORT: u16 = 8991;

/// Resolve `web_addr` from config.toml.
///
/// Absent means on, on loopback. An empty string means off. The endpoint
/// reports what is playing, which is the user's listening history, so the
/// default is the narrowest thing that is still useful; widening it is one
/// line of config and has to be asked for deliberately.
fn web_addr(toml: Option<&toml::Value>) -> Option<SocketAddr> {
    let raw = toml.and_then(|v| v.get("web_addr")).map(|v| {
        if let Some(s) = v.as_str() {
            s.to_string()
        } else if let Some(i) = v.as_integer() {
            i.to_string()
        } else {
            String::new()
        }
    });
    let addr = match raw {
        Some(s) if s.trim().is_empty() => return None,
        Some(s) => s,
        None => format!("127.0.0.1:{WEB_PORT}"),
    };
    // A bare port is a convenience: `web_addr = 8992` means loopback on 8992,
    // never 0.0.0.0, so the shorthand cannot accidentally expose the daemon.
    let addr = if addr.chars().all(|c| c.is_ascii_digit()) {
        format!("127.0.0.1:{addr}")
    } else {
        addr
    };
    match addr.parse() {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("gtmd: ignoring web_addr {addr:?}: {e}");
            Some(
                format!("127.0.0.1:{WEB_PORT}")
                    .parse()
                    .expect("loopback addr"),
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_file: PathBuf,
    pub library_paths: Vec<PathBuf>,
    pub log_file: Option<PathBuf>,
    pub test_mode: bool,
    pub audio_backend: AudioBackendKind,
    /// Permit the daemon to physically delete audio files (and their `.lrc`
    /// sidecars) when a track is removed from the library. When disabled, the
    /// remove action is refused with an error instead of touching the file.
    /// Defaults to allowing deletion (matches pre-flag behaviour).
    pub allow_delete_files: bool,
    /// Artwork source preference, read from the TUI's config.toml.
    pub cover_provider: CoverProvider,
    /// Combined on-disk cover cache budget in bytes, from `cover_cache_mb`.
    pub cover_cache_bytes: u64,
    /// Address for the read-only JSON status endpoint, from `web_addr`.
    ///
    /// Defaults to loopback so the endpoint is reachable by the user's own
    /// scripts and nothing else. Set it to `0.0.0.0:PORT` to expose the
    /// current track to the LAN, or to an empty string to disable it.
    pub web_addr: Option<SocketAddr>,
    /// Discord application id for Rich Presence, from `discord_app_id`.
    ///
    /// `None` disables presence. An id that Discord does not recognise simply
    /// never gets a connection, so a wrong value costs a log line.
    pub discord_id: Option<String>,
}

#[derive(Parser, Debug)]
#[command(name = "gtmd", about = "gtm background audio daemon")]
pub struct DaemonArgs {
    #[arg(long, help = "Unix socket path", value_hint = clap::ValueHint::AnyPath)]
    pub socket: Option<String>,

    #[arg(long, help = "Library database path", value_hint = clap::ValueHint::FilePath)]
    pub library: Option<String>,

    #[arg(long, help = "Config directory path", value_hint = clap::ValueHint::DirPath)]
    pub config: Option<String>,

    #[arg(short, long, help = "Enable verbose logging")]
    pub verbose: bool,

    #[arg(long, help = "Test mode (ephemeral socket, no daemonize)")]
    pub test_mode: bool,

    #[arg(long, help = "Audio backend", value_parser = ["rodio", "pulseaudio"])]
    pub backend: Option<String>,

    /// Write a shell completion script to stdout and exit
    ///
    /// Hidden: packaging calls it. Generated from this struct, which is the
    /// real parser, so the script cannot drift from the flags accepted here.
    #[arg(long, value_name = "SHELL", hide = true)]
    pub completions: Option<clap_complete::Shell>,
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
            resolve_command_socket()
        };

        let socket_pulse_path = if let Some(ref s) = args.socket {
            // Mirror the client and `resolve_pulse_socket`: replace the socket
            // extension with `pulse` so both ends agree on the path.
            let mut p = PathBuf::from(s);
            p.set_extension("pulse");
            p
        } else {
            resolve_pulse_socket()
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
            _ if is_termux() => {
                eprintln!(
                    "gtmd: Termux detected: using the PulseAudio backend. \
                     The server will be started automatically if needed."
                );
                AudioBackendKind::PulseAudio
            }
            #[cfg(not(feature = "pulseaudio"))]
            _ if is_termux() => {
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
        library_paths.extend(termux_music_dirs());

        let state_file = data_dir.join("state.json");

        // `cover_provider` lives in the same config.toml the gtm TUI edits.
        // Known keys are honored; anything unrecognized falls back to Auto.
        let toml = std::fs::read_to_string(config_dir.join("config.toml"))
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok());
        let cover_provider = toml
            .as_ref()
            .and_then(|v| {
                v.get("cover_provider")
                    .and_then(|p| p.as_str())
                    .map(CoverProvider::from_str_lossy)
            })
            .unwrap_or_default();
        let cover_cache_bytes = toml
            .as_ref()
            .and_then(|v| v.get("cover_cache_mb").and_then(|m| m.as_integer()))
            .filter(|mb| *mb > 0)
            .map(|mb| (mb as u64) * 1024 * 1024)
            .unwrap_or(crate::cover::DISK_CACHE_DEFAULT);
        let web_addr = web_addr(toml.as_ref());
        let discord_id = discord_id(toml.as_ref());

        DaemonConfig {
            socket_path,
            socket_pulse_path,
            config_dir,
            cache_dir,
            data_dir,
            state_file,
            library_paths,
            log_file,
            test_mode: args.test_mode,
            audio_backend,
            allow_delete_files: true,
            cover_provider,
            cover_cache_bytes,
            web_addr,
            discord_id,
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
        if let Some(ref log) = self.log_file
            && let Some(parent) = log.parent()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

/// Resolve `discord_app_id` from config.toml.
fn discord_id(toml: Option<&toml::Value>) -> Option<String> {
    let raw = match toml.and_then(|v| v.get("discord_app_id"))? {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        _ => return None,
    };
    let id = raw.trim();
    // Discord's ids are numeric. Anything else is a paste error, and refusing
    // it here is better than a handshake that silently never connects.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        eprintln!("gtmd: ignoring discord_app_id {id:?}: expected a numeric application id");
        return None;
    }
    Some(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> toml::Value {
        toml::from_str(s).unwrap()
    }

    /// Loopback by default, because the endpoint reports listening history.
    #[test]
    fn web_defaults_to_loopback() {
        assert_eq!(web_addr(None), Some("127.0.0.1:8991".parse().unwrap()));
        assert_eq!(
            web_addr(Some(&val("[web]\n"))),
            Some("127.0.0.1:8991".parse().unwrap())
        );
    }

    /// An empty value is how you turn it off.
    #[test]
    fn web_off_when_empty() {
        assert_eq!(web_addr(Some(&val("web_addr = \"\""))), None);
    }

    /// A bare port is a convenience and must never widen the bind: a user who
    /// types a number wants a different port, not a LAN-exposed daemon.
    #[test]
    fn bare_port_stays_on_loopback() {
        assert_eq!(
            web_addr(Some(&val("web_addr = 9000"))),
            Some("127.0.0.1:9000".parse().unwrap())
        );
    }

    /// Widening is allowed, but only when asked for explicitly.
    #[test]
    fn web_addr_honours_an_explicit_bind() {
        assert_eq!(
            web_addr(Some(&val("web_addr = \"0.0.0.0:8991\""))),
            Some("0.0.0.0:8991".parse().unwrap())
        );
    }

    /// A typo must not take the endpoint down silently, and must not turn
    /// into a wildcard bind either.
    #[test]
    fn bad_web_addr_falls_back_to_loopback() {
        assert_eq!(
            web_addr(Some(&val("web_addr = \"not-an-addr\""))),
            Some("127.0.0.1:8991".parse().unwrap())
        );
    }

    #[test]
    fn discord_off_by_default() {
        assert_eq!(discord_id(None), None);
        assert_eq!(discord_id(Some(&val("[audio]\n"))), None);
    }

    #[test]
    fn discord_accepts_a_numeric_id() {
        let id = "1554792961844445225".to_string();
        assert_eq!(
            discord_id(Some(&val(&format!("discord_app_id = {id}")))),
            Some(id.clone())
        );
        assert_eq!(
            discord_id(Some(&val(&format!("discord_app_id = \"{id}\"")))),
            Some(id)
        );
    }

    /// A non-numeric id is a paste error, and a handshake that never connects
    /// is a much worse way to learn that than a refusal at load.
    #[test]
    fn discord_rejects_a_non_numeric_id() {
        assert_eq!(discord_id(Some(&val("discord_app_id = \"abc\""))), None);
        assert_eq!(discord_id(Some(&val("discord_app_id = \"\""))), None);
    }
}
