// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon entry point: CLI parsing, logging setup, and daemon launch
//
// This is free software released under the GPL-3.0 license.

use clap::Parser;
use tracing_subscriber::EnvFilter;

use gtmd::config::{DaemonArgs, DaemonConfig};
use gtmd::daemon::Daemon;

fn print_version() {
    let ver = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    println!(
        "gtmd {ver}\n\
         Copyright (C) 2026 prjctimg <prjctimg@outlook.com>\n\
         Website: https://prjctimg.me\n\
         License GPL-3.0\n\
         This is free software: you are free to change and redistribute it.\n\
         There is NO WARRANTY, to the extent permitted by law."
    );
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        print_version();
        return;
    }

    let args = DaemonArgs::parse();

    let config = DaemonConfig::load(&args);

    if let Err(e) = config.create_dirs() {
        eprintln!("failed to create daemon directories: {e}");
        std::process::exit(1);
    }

    let log_file = config.log_file.as_ref().map(|p| p.as_path());

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

    match Daemon::new(config) {
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
