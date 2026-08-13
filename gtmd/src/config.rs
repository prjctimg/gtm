use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioBackendKind {
    Rodio,
}

impl Default for AudioBackendKind {
    fn default() -> Self {
        Self::Rodio
    }
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub library_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
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

    #[arg(long, help = "Audio backend (rodio)")]
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
        } else if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime).join("gtmd.socket")
        } else {
            std::env::temp_dir().join("gtmd.socket")
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

        let _unused_backend = args.backend.as_deref();
        let audio_backend = AudioBackendKind::Rodio;

        DaemonConfig {
            socket_path,
            library_path,
            config_dir,
            cache_dir,
            data_dir,
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
        if let Some(ref log) = self.log_file {
            if let Some(parent) = log.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(())
    }
}
