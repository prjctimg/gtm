use std::process::Command;

fn main() {
    // Git commit SHA
    let git_sha = Command::new("git")
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

    // Detect linker and mold availability for Linux targets
    if cfg!(target_os = "linux") && !cfg!(target_env = "musl") {
        let has_clang = Command::new("clang")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let has_mold = Command::new("mold")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let has_gcc = Command::new("gcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let (linker, use_mold) = if has_clang && has_mold {
            ("clang", true)
        } else if has_gcc && has_mold {
            ("gcc", true)
        } else if has_clang {
            ("clang", false)
        } else {
            ("gcc", false)
        };

        println!("cargo:rustc-env=GTM_LINKER={}", linker);
        println!("cargo:rustc-env=GTM_USE_MOLD={}", use_mold);
        println!(
            "cargo:rustc-link-arg=-fuse-ld={}",
            if use_mold { "mold" } else { "ld" }
        );
    }

    // Re-run if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
}
