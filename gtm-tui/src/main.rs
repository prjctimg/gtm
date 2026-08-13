mod app;
mod ui;

use std::error::Error;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "gtm-tui", about = "GTM Terminal UI")]
struct Args {
    #[arg(long, default_value = "/run/user/1000/gtmd.socket")]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    color_eyre::install()?;
    let args = Args::parse();
    let mut terminal = ratatui::init();
    let res = app::App::new(&args.socket).await?.run(&mut terminal).await;
    ratatui::restore();
    res
}
