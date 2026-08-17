use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use tracing::{info, warn};

const GITHUB_REPO: &str = "prjctimg/gtm.rs";
const CHECK_INTERVAL_SECS: u64 = 3600 * 6;

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse_version(v: &str) -> Vec<u32> {
    v.trim_start_matches('v')
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn platform_target() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-linux"),
        ("linux", "aarch64") => Some("aarch64-linux"),
        ("macos", "aarch64") => Some("aarch64-darwin"),
        ("android", _) => Some("aarch64-android"),
        _ => None,
    }
}

fn asset_name(target: &str) -> String {
    format!("gtm-{}.tar.gz", target)
}

pub async fn check_for_update() -> Option<(String, String)> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let client = reqwest::Client::new();
    let resp = match client
        .get(&url)
        .header("User-Agent", format!("gtm/{}", current_version()))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("update check failed: {}", e);
            return None;
        }
    };
    let release: GitHubRelease = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            warn!("update check parse failed: {}", e);
            return None;
        }
    };

    let latest = release.tag_name.trim_start_matches('v');
    let current = current_version();
    if parse_version(latest) <= parse_version(current) {
        return None;
    }

    let target = platform_target()?;
    let expected_asset = asset_name(target);
    let asset = release.assets.iter().find(|a| a.name == expected_asset)?;
    Some((latest.to_string(), asset.browser_download_url.clone()))
}

pub async fn perform_update() -> Result<String, String> {
    let (version, url) = check_for_update().await.ok_or("no update available")?;
    info!("updating to v{}", version);

    let client = reqwest::Client::new();
    let bytes = client
        .get(&url)
        .header("User-Agent", format!("gtm/{}", current_version()))
        .send()
        .await
        .map_err(|e| format!("download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("download body failed: {}", e))?;

    let decoder = GzDecoder::new(&bytes[..]);
    let mut archive = Archive::new(decoder);

    let tmp_dir = std::env::temp_dir().join(format!("gtm-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("tmp dir: {}", e))?;

    archive
        .unpack(&tmp_dir)
        .map_err(|e| format!("extract: {}", e))?;

    let extracted = std::fs::read_dir(&tmp_dir)
        .map_err(|e| format!("read tmp: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .ok_or("no extracted directory found")?
        .path();

    let prefix = if gtm_core::is_termux() {
        std::env::var("PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/data/data/com.termux/files/usr"))
    } else {
        PathBuf::from("/usr/local")
    };

    let bin_dir = prefix.join("bin");
    let man_dir = prefix.join("share").join("man").join("man1");
    let completions_dir = prefix.join("share");

    let bin_src = extracted.join("bin");
    if bin_src.exists() {
        for name in &["gtm", "gtmd"] {
            let src = bin_src.join(name);
            let dst = bin_dir.join(name);
            if src.exists() {
                std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {}", name, e))?;
                let mut perm = std::fs::metadata(&dst)
                    .map_err(|e| format!("stat {}: {}", name, e))?
                    .permissions();
                perm.set_mode(0o755);
                std::fs::set_permissions(&dst, perm)
                    .map_err(|e| format!("chmod {}: {}", name, e))?;
                info!("updated {}", dst.display());
            }
        }
    }

    let man_src = extracted.join("man").join("man1");
    if man_src.exists() {
        let _ = std::fs::create_dir_all(&man_dir);
        for entry in std::fs::read_dir(&man_src)
            .unwrap_or_else(|_| std::fs::read_dir("/nonexistent").unwrap())
        {
            if let Ok(entry) = entry {
                let dst = man_dir.join(entry.file_name());
                let _ = std::fs::copy(entry.path(), &dst);
            }
        }
        info!("updated manpages");
    }

    let comp_src = extracted.join("completions");
    if comp_src.exists() {
        let bash_dir = completions_dir.join("bash-completion").join("completions");
        let _ = std::fs::create_dir_all(&bash_dir);
        for name in &["gtm", "gtmd"] {
            let src = comp_src.join(format!("{}.bash", name));
            if src.exists() {
                let _ = std::fs::copy(&src, bash_dir.join(name));
            }
        }
        let zsh_dir = completions_dir.join("zsh").join("vendor-completions");
        let _ = std::fs::create_dir_all(&zsh_dir);
        for name in &["gtm", "gtmd"] {
            let src = comp_src.join(format!("_{}", name));
            if src.exists() {
                let _ = std::fs::copy(&src, zsh_dir.join(format!("_{}", name)));
            }
        }
        let fish_dir = completions_dir.join("fish").join("vendor_completions.d");
        let _ = std::fs::create_dir_all(&fish_dir);
        for name in &["gtm", "gtmd"] {
            let src = comp_src.join(format!("{}.fish", name));
            if src.exists() {
                let _ = std::fs::copy(&src, fish_dir.join(format!("{}.fish", name)));
            }
        }
        info!("updated completions");
    }

    let svc_src = extracted.join("systemd").join("gtmd.service");
    if svc_src.exists() {
        let svc_dst = prefix
            .join("lib")
            .join("systemd")
            .join("user")
            .join("gtmd.service");
        if let Some(parent) = svc_dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(&svc_src, &svc_dst);
        info!("updated systemd service");
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);

    restart_daemon();

    Ok(version)
}

fn restart_daemon() {
    if Command::new("systemctl")
        .args(["--user", "restart", "gtmd"])
        .output()
        .is_ok()
    {
        info!("restarted via systemd");
        return;
    }
    warn!("please restart gtmd manually");
}

pub async fn update_check_loop() {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(CHECK_INTERVAL_SECS)).await;
        if let Some((version, _url)) = check_for_update().await {
            info!("update available: v{}", version);
            match perform_update().await {
                Ok(v) => info!("updated to v{}", v),
                Err(e) => warn!("update failed: {}", e),
            }
        }
    }
}
