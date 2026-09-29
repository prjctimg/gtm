// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Browser launch and credential-masking helpers shared by the provider links
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

/// Best-effort browser open for an OAuth authorize URL. Tries the OS default
/// opener (via `webbrowser`) then common launchers, each timeout-guarded so a
/// wedged launcher never blocks a runtime worker or the UI.
pub async fn open_browser(url: &str) -> bool {
    // Prefer the OS default browser opener, which is cross-platform. Guard
    // it with a timeout: a wedged `xdg-open`-style launcher must not leave
    // the flow looking dead.
    let opened = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::task::spawn_blocking({
            let url = url.to_string();
            move || webbrowser::open(&url)
        })
        .await
    })
    .await;
    if let Ok(Ok(Ok(_))) = opened {
        return true;
    }
    // Fallback to common launchers when the `webbrowser` crate can't
    // resolve one (e.g. minimal containers / WSL).
    for prog in ["xdg-open", "open", "start"] {
        if let Ok(st) = tokio::process::Command::new(prog)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            && st.success()
        {
            return true;
        }
    }
    false
}

/// Render a credential safe to write to a log or show in a notification.
///
/// A fixed-width mask, never a truncation. An earlier version showed the last
/// version sliced with `&s[..2]` and panicked on a pasted smart quote, and
/// removing the slicing removes that class of bug rather than fixing one case
/// of it.
pub fn mask_credential(s: &str) -> String {
    if s.chars().count() <= 6 {
        return "****".to_string();
    }
    "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string()
}
