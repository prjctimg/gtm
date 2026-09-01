// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// This is free software released under the GPL-3.0 license.

// Build-script for gtmd.
//
// Extra duties beyond plain cargo metadata:
//  * Auto-detect a Termux build and set `gtm_termux` so the crate can adapt
//    (e.g. prefer the PulseAudio backend).
//  * If a Termux build is detected without the `pulseaudio` feature, emit a
//    clear, actionable build warning instead of failing at runtime with an
//    obscure audio error.

use std::env;

fn main() {
    // Termux cross-builds target `aarch64-linux-android`; the environment also
    // sets $PREFIX and/or $TERMUX_VERSION when building on-device.
    let target = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let in_termux =
        target == "android" || env::var("PREFIX").is_ok() || env::var("TERMUX_VERSION").is_ok();

    if in_termux {
        println!("cargo:rustc-cfg=gtm_termux");
        println!("cargo:rustc-check-cfg=cfg(gtm_termux)");
        let has_pulse = env::var("CARGO_FEATURE_PULSEAUDIO").is_ok();
        if !has_pulse {
            println!(
                "cargo:warning=Termux detected: enable the `pulseaudio` backend with \
                 `cargo build --features pulseaudio` (the Makefile `termux` and `termux-deb` \
                 targets do this for you)."
            );
        }
    }

    println!("cargo:rerun-if-env-changed=PREFIX");
    println!("cargo:rerun-if-env-changed=TERMUX_VERSION");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-changed=build.rs");
}
