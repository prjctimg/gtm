use std::env;
use std::process::Command;

// Shell-completion generation for release packaging. The structs + generator
// live in build/completions.rs (pulled in verbatim from the former
// `release-gen` crate) and only run when GTM_GEN_COMPLETIONS is set.
mod completions {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/build/completions.rs"));
}

/// Look up a package's version in the workspace Cargo.lock.
fn locked_version(pkg: &str) -> Option<String> {
    let lock = std::fs::read_to_string("Cargo.lock").ok()?;
    let mut in_pkg = false;
    for line in lock.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("name = ") {
            let name = name.trim_matches('"');
            in_pkg = name == pkg;
        } else if in_pkg && let Some(ver) = line.strip_prefix("version = ") {
            return Some(ver.trim_matches('"').to_string());
        }
    }
    None
}

fn main() {
    // Termux cross-builds target `aarch64-linux-android`; the environment also
    // sets $PREFIX and/or $TERMUX_VERSION when building on-device. If that is
    // detected without the `pulseaudio` feature, warn now instead of failing at
    // runtime with an obscure audio error.
    let target = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let in_termux =
        target == "android" || env::var("PREFIX").is_ok() || env::var("TERMUX_VERSION").is_ok();
    if in_termux && env::var("CARGO_FEATURE_PULSEAUDIO").is_err() {
        println!(
            "cargo:warning=Termux detected: enable the `pulseaudio` backend with \
             `cargo build --features pulseaudio` (the Makefile `termux` and `termux-deb` \
             targets do this for you)."
        );
    }
    println!("cargo:rerun-if-env-changed=PREFIX");
    println!("cargo:rerun-if-env-changed=TERMUX_VERSION");

    // Git commit SHA - try env var first (for CI), then git rev-parse
    let git_sha = std::env::var("VERGEN_GIT_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8(o.stdout)
                            .ok()
                            .map(|s| s.trim().to_string())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VERGEN_GIT_SHA={}", git_sha);

    // Build date (YYYY-MM-DD)
    let build_date = Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VERGEN_BUILD_DATE={}", build_date);

    // Rust compiler version
    let rust_ver = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let stdout = String::from_utf8(o.stdout).unwrap_or_default();
                stdout.split_whitespace().nth(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VERGEN_RUSTC_SEMVER={}", rust_ver);

    // Key library versions (from Cargo.lock) surfaced into the About window.
    for (name, env) in [
        ("ratatui", "GTM_RATATUI_VERSION"),
        ("rodio", "GTM_RODIO_VERSION"),
        ("symphonia", "GTM_SYMPHONIA_VERSION"),
    ] {
        let v = locked_version(name).unwrap_or_else(|| "unknown".into());
        println!("cargo:rustc-env={}={}", env, v);
    }

    // Linker provenance: mold when enabled, otherwise let the default apply.
    if cfg!(target_os = "linux")
        && !cfg!(target_env = "musl")
        && Command::new("mold")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        println!("cargo:rustc-env=GTM_LINKER=mold");
    } else {
        println!("cargo:rustc-env=GTM_LINKER=default");
    }

    // Speed up local Linux builds by using mold when it's installed. When mold
    // is absent we emit nothing and let rustc pick the default linker, so the
    // build never fails on minimal CI runners that lack mold (or where the
    // bundled cc only links via lld).
    if cfg!(target_os = "linux")
        && !cfg!(target_env = "musl")
        && Command::new("mold")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        println!("cargo:rustc-link-arg=-fuse-ld=mold");
        println!("cargo:rustc-env=GTM_USE_MOLD=true");
    }

    // Re-run if the CI-injected commit changes so a restored cargo cache can
    // never bake a stale "unknown" or previous-commit SHA into the binary.
    println!("cargo:rerun-if-env-changed=VERGEN_GIT_SHA");
    // Re-run if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");

    // Release packaging: regenerate shell completions when the env var is set
    // (used by `make completions` and the GitHub release workflow). Resolves
    // relative outdirs against the workspace root, mirroring how release-gen
    // behaved when run from the repository root.
    println!("cargo:rerun-if-env-changed=GTM_GEN_COMPLETIONS");
    if let Ok(outdir) = env::var("GTM_GEN_COMPLETIONS")
        && !outdir.trim().is_empty()
    {
        completions::generate(&outdir);
    }
}
