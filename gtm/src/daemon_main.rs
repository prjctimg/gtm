// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon binary bundled in the gtm crate so a single `cargo install gtm`
// ships both the TUI (disablable via --no-default-features) and gtmd.
//
// This is free software released under the GPL-3.0 license.

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

    gtmd::run().await;
}
