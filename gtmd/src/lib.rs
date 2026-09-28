// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon library root: re-exports all daemon submodules
//
// This is free software released under the GPL-3.0 license.

use clap::Parser;
use tracing_subscriber::EnvFilter;

pub mod cleaner;
pub mod config;
pub mod cover;
pub mod daemon;
pub mod deferred_mixer;
pub mod library;
pub mod network;
pub mod providers;
pub mod queue;
pub mod remote;
pub mod tags;

// Re-exported at the crate root so `crate::spotify::…` and friends keep
// resolving; the implementation now lives under one directory per provider.
pub use providers::lrclib as lyrics;
#[cfg(feature = "youtube")]
pub use providers::youtube;
pub use providers::{charts, deezer, lastfm, musicbrainz, podcast, radio, spotify};

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
    // rspotify logs each outgoing request at `info`, Debug-printing the whole
    // request builder — which carries `Authorization: Bearer <access token>`.
    // The log file is not private, so that wrote a live credential other users
    // on the machine could read, once per API call. Capping the target at
    // `warn` costs the request trace, which said nothing we act on. `RUST_LOG`
    // replaces the directive list wholesale, so an operator who asks for that
    // target back still gets it.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{log_level},rspotify_http=warn")));

    if let Some(path) = log_file {
        let file = match std::fs::File::create(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("failed to create log file {path:?}: {e}");
                std::process::exit(1);
            }
        };
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            // A file is never a terminal: without this the subscriber's own
            // detection enables colour and every line lands wrapped in escapes.
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
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
