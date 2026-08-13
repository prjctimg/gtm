use clap::Parser;
use tracing_subscriber::EnvFilter;

use gtmd::config::{DaemonArgs, DaemonConfig};
use gtmd::daemon::Daemon;

#[tokio::main]
async fn main() {
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
