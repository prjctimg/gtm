// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Centralised logging to file — replaces eprintln! in library code
//
// This is free software released under the GPL-3.0 license.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;

static LOG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
static LOG_FILE: OnceLock<std::sync::Mutex<std::fs::File>> = OnceLock::new();

fn log_path() -> &'static std::path::PathBuf {
    LOG_PATH.get_or_init(|| {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            let p = std::path::PathBuf::from(xdg).join("gtm");
            let _ = std::fs::create_dir_all(&p);
            p.join("gtm.log")
        } else if let Ok(home) = std::env::var("HOME") {
            let p = std::path::PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("gtm");
            let _ = std::fs::create_dir_all(&p);
            p.join("gtm.log")
        } else {
            std::env::temp_dir().join("gtm.log")
        }
    })
}

fn log_file() -> &'static std::sync::Mutex<std::fs::File> {
    LOG_FILE.get_or_init(|| {
        let path = log_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|_| std::fs::File::create(path).expect("cannot create log file"));
        std::sync::Mutex::new(file)
    })
}

/// Write a timestamped line to the log file.
pub fn log(msg: &str) {
    use chrono::Local;
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{ts}] {msg}\n");
    if let Ok(mut f) = log_file().lock() {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

/// Return the path to the log file.
pub fn log_file_path() -> std::path::PathBuf {
    log_path().clone()
}

/// Redirect process stderr to the log file.
/// Returns the original stderr fd for later restoration.
pub fn redirect_stderr_to_log() -> std::os::unix::io::RawFd {
    use std::os::unix::io::AsRawFd;
    let path = log_path();
    let file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(f) => f,
        Err(_) => return -1,
    };
    let original_fd = unsafe { libc::dup(libc::STDERR_FILENO) };
    let raw = file.as_raw_fd();
    unsafe {
        libc::dup2(raw, libc::STDERR_FILENO);
    }
    original_fd
}
