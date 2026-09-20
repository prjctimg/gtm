// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon library root: re-exports all daemon submodules
//
// This is free software released under the GPL-3.0 license.

use clap::Parser;
use tracing_subscriber::EnvFilter;

pub mod charts;
pub mod cleaner;
pub mod config;
pub mod cover;
pub mod daemon;
pub mod deezer;
pub mod deferred_mixer;
pub mod lastfm;
pub mod library;
pub mod lyrics;
pub mod musicbrainz;
pub mod network;
pub mod oauth;
pub mod podcast;
pub mod queue;
pub mod radio;
pub mod remote;
pub mod spotify;
pub mod stream;
pub mod subsonic;
pub mod tags;
#[cfg(feature = "youtube")]
pub mod youtube;

pub use config::{DaemonArgs, DaemonConfig};
pub use daemon::Daemon;

pub async fn run() {
    let args = DaemonArgs::parse();
    let config = DaemonConfig::load(&args);

    if let Err(e) = config.create_dirs() {
        eprintln!("failed to create daemon directories: {e}");
        std::process::exit(1);
    }

    let log_file = config.log_file.as_deref();
    let log_level = if args.verbose { "debug" } else { "info" };

    if let Some(path) = log_file {
        let file = match std::fs::File::create(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("failed to create log file {path:?}: {e}");
                std::process::exit(1);
            }
        };
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
            )
            .with_writer(std::sync::Mutex::new(file))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
            )
            .init();
    }

    tracing::info!("starting gtm daemon");

    match Daemon::new(config).await {
        Ok(mut daemon) => {
            if let Err(e) = daemon.run().await {
                tracing::error!("daemon exited: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            tracing::error!("failed to create daemon: {e}");
            std::process::exit(1);
        }
    }
}
