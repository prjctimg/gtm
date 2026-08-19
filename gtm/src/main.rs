// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// gtm music player: single binary, TUI + CLI modes
//
// This is free software released under the GPL-3.0 license.

use clap::{CommandFactory, Parser};

use gtm::cli;
use gtm::ui;

fn print_version() {
    let ver = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    println!(
        "gtm {ver}\n\
         Copyright (C) 2026 - present prjctimg <prjctimg@outlook.com>\n\
         Website: https://prjctimg.me\n\
         License GPL-3.0\n\
         This is free software: you are free to change and redistribute it.\n\
         There is NO WARRANTY, to the extent permitted by law."
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        print_version();
        return;
    }

    let args = cli::Args::parse();

    if let Some(ref cmd) = args.command {
        // A subcommand was given → run in CLI mode, dispatch directly
        cli::run(args.socket, args.json, args.verbose, cmd);
    } else if args.cli {
        // --cli flag with no subcommand → print CLI help
        let mut cmd = cli::Args::command();
        cmd.print_help().unwrap();
        println!();
    } else {
        // No subcommand, no --cli → launch the TUI
        let res = ui::run_tui(args.socket);
        if let Err(e) = res {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
