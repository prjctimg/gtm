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
                // "rustc 1.XX.Y (hash date)" → "1.XX.Y"
                stdout.split_whitespace().nth(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=VERGEN_RUSTC_SEMVER={}", rust_ver);

    // Re-run if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
}
