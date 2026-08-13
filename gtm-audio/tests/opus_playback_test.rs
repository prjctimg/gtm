use gtm_audio::AudioMixer;
use std::time::Duration;

fn find_opus_file() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let audio_dir = format!("{}/.local/share/gtm/audio", home);
    let mut files: Vec<_> = std::fs::read_dir(&audio_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "opus").unwrap_or(false))
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    files.sort();
    files.into_iter().next()
}

#[test]
fn test_mixer_load_opus() {
    let path = match find_opus_file() {
        Some(p) => p,
        None => {
            eprintln!("no opus files found, skipping test");
            return;
        }
    };

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("audio device init failed: {e}, skipping");
            return;
        }
    };

    mixer.load_active(&path, 0.0).unwrap();
    assert!(mixer.duration() > 0.0, "duration should be > 0");
    assert_eq!(mixer.current_position(), 0.0);
    assert!(!mixer.is_playing());
    assert_eq!(mixer.volume(), 100);

    mixer.play().unwrap();
    assert!(mixer.is_playing());

    std::thread::sleep(Duration::from_millis(300));
    let ev = mixer.poll().unwrap();
    assert!(ev.is_some(), "expected a Position event after playing");

    let pos = mixer.current_position();
    assert!(pos > 0.0, "position should advance");

    mixer.set_volume(50).unwrap();
    assert_eq!(mixer.volume(), 50);

    mixer.stop().unwrap();
    assert!(!mixer.is_playing());
    assert_eq!(mixer.current_position(), 0.0);
}

#[test]
fn test_mixer_load_play_pause_stop() {
    let path = match find_opus_file() {
        Some(p) => p,
        None => {
            eprintln!("no opus files found, skipping test");
            return;
        }
    };

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("audio device init failed: {e}, skipping");
            return;
        }
    };

    mixer.load_active(&path, 0.0).unwrap();
    mixer.play().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(mixer.is_playing());

    mixer.pause().unwrap();
    assert!(!mixer.is_playing());
    let paused_pos = mixer.current_position();
    std::thread::sleep(Duration::from_millis(200));
    assert!((mixer.current_position() - paused_pos).abs() < 0.5);

    mixer.play().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    mixer.stop().unwrap();
}

#[test]
fn test_mixer_seek_opus() {
    let path = match find_opus_file() {
        Some(p) => p,
        None => {
            eprintln!("no opus files found, skipping test");
            return;
        }
    };

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("audio device init failed: {e}, skipping");
            return;
        }
    };

    mixer.load_active(&path, 0.0).unwrap();
    let dur = mixer.duration();
    if dur <= 5.0 {
        eprintln!("opus test file too short ({dur}s) for seek test, skipping");
        mixer.stop().unwrap();
        return;
    }

    if mixer.seek(10.0).is_err() {
        eprintln!("seek not supported for this format, skipping");
        mixer.stop().unwrap();
        return;
    }
    let seek_pos = mixer.current_position();
    assert!((seek_pos - 10.0).abs() < 2.0);

    mixer.play().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(mixer.current_position() >= seek_pos);

    mixer.stop().unwrap();
}

#[test]
fn test_mixer_poll_finished() {
    let path = match find_opus_file() {
        Some(p) => p,
        None => {
            eprintln!("no opus files found, skipping test");
            return;
        }
    };

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("audio device init failed: {e}, skipping");
            return;
        }
    };

    mixer.load_active(&path, 0.0).unwrap();
    mixer.play().unwrap();
    std::thread::sleep(Duration::from_millis(200));

    mixer.stop().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let ev = mixer.poll().unwrap();
    assert!(ev.is_none(), "poll should return None after stop");
}

#[test]
fn test_mixer_nonexistent_file() {
    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("audio device init failed: {e}, skipping");
            return;
        }
    };

    let result = mixer.load_active("/nonexistent/opus/file.opus", 0.0);
    assert!(result.is_err());
}
