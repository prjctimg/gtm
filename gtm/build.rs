use std::process::Command;
use std::str;

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

    // Generate manpages from docs/man/*.1.md at build time
    let out = Command::new("bash")
        .arg("scripts/build/manpages.sh")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();
    if let Ok(output) = out {
        if output.status.success() {
            let stdout = str::from_utf8(&output.stdout).unwrap_or("");
            for line in stdout.lines() {
                eprintln!("manpage: {}", line);
            }
            // Set env vars so the binary can find the manpages at runtime
            // (the player reads them from a known relative path)
            // We set MANPAGES_DIR so the :man command can locate them
            if let Some(dir_line) = stdout.lines().find(|l| l.starts_with("Manpages generated")) {
                // The script prints "Manpages generated in X/" - extract the path
                if let Some(start) = dir_line.find("in ") {
                    let path = &dir_line[start + 3..];
                    println!("cargo:rustc-env=MANPAGES_DIR={}", path);
                }
            }
        } else {
            eprintln!("manpage generation failed: {}", str::from_utf8(&output.stderr).unwrap_or(""));
        }
    } else {
        eprintln!("manpage generation: script not found, skipping");
    }
}
