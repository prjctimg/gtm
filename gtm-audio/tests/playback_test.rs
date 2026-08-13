use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gtm_audio::AudioMixer;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn create_test_wav(path: &std::path::Path, duration_secs: f64) {
    let sample_rate: u32 = 44100;
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let bytes_per_sample = bits_per_sample as u16 / 8;
    let num_samples = (sample_rate as f64 * duration_secs) as u64 * channels as u64;
    let data_size = num_samples * bytes_per_sample as u64;
    let file_size = 36 + data_size;

    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(file_size as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    wav.extend_from_slice(b"fmt ");
    let fmt_size: u32 = 16;
    wav.extend_from_slice(&fmt_size.to_le_bytes());
    let audio_format: u16 = 1;
    wav.extend_from_slice(&audio_format.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * bytes_per_sample;
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());

    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());

    // Fill with a simple sine wave (440 Hz)
    for i in 0..num_samples {
        let sample = (i as f64 * 440.0 * 2.0 * std::f64::consts::PI / sample_rate as f64).sin();
        let amplitude = (i16::MAX as f64 * 0.3) as i16;
        let val = (sample * amplitude as f64) as i16;
        wav.extend_from_slice(&val.to_le_bytes());
    }

    std::fs::write(path, &wav).unwrap();
}

fn test_wav_path() -> std::path::PathBuf {
    let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut p = std::env::temp_dir();
    p.push(format!("gtm_audio_test_{n}.wav"));
    p
}

#[test]
fn test_mixer_load_play_pause_stop() {
    let wav_path = test_wav_path();
    create_test_wav(&wav_path, 3.0);

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("no audio device available: {e}, skipping test");
            let _ = std::fs::remove_file(&wav_path);
            return;
        }
    };

    mixer.load_active(wav_path.to_str().unwrap(), 0.0).unwrap();
    assert_eq!(mixer.current_position(), 0.0);
    assert!(mixer.duration() > 0.0);
    assert!(!mixer.is_playing());
    assert_eq!(mixer.volume(), 100);

    mixer.play().unwrap();
    assert!(mixer.is_playing());

    // Poll for position updates
    std::thread::sleep(Duration::from_millis(200));
    let ev = mixer.poll().unwrap();
    assert!(ev.is_some(), "expected a Position event after playing");
    if let Some(gtm_audio::AudioEvent::Position(pos)) = ev {
        assert!(pos > 0.0, "position should advance after playing");
        assert!(pos <= mixer.duration());
    }

    let pos_after_play = mixer.current_position();
    assert!(pos_after_play > 0.0);

    // Seek forward
    mixer.seek(1.5).unwrap();
    let seek_pos = mixer.current_position();
    assert!((seek_pos - 1.5).abs() < 0.1);

    // Pause
    mixer.pause().unwrap();
    assert!(!mixer.is_playing());
    let paused_pos = mixer.current_position();

    // Small wait then verify position hasn't changed much
    std::thread::sleep(Duration::from_millis(300));
    assert!((mixer.current_position() - paused_pos).abs() < 1.0);

    // Resume
    mixer.play().unwrap();
    assert!(mixer.is_playing());
    std::thread::sleep(Duration::from_millis(200));
    assert!(mixer.current_position() >= paused_pos);

    // Change volume
    mixer.set_volume(50).unwrap();
    assert_eq!(mixer.volume(), 50);

    // Stop
    mixer.stop().unwrap();
    assert!(!mixer.is_playing());
    assert_eq!(mixer.current_position(), 0.0);

    // Load at offset
    mixer.load_active(wav_path.to_str().unwrap(), 1.0).unwrap();
    let load_pos = mixer.current_position();
    assert!((load_pos - 1.0).abs() < 0.1);
    assert!(mixer.duration() > 1.0);

    // Clean stop
    mixer.stop().unwrap();

    std::fs::remove_file(&wav_path).ok();
}

#[test]
fn test_mixer_poll_detects_finished() {
    let wav_path = test_wav_path();
    create_test_wav(&wav_path, 0.5);

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("no audio device available: {e}, skipping test");
            let _ = std::fs::remove_file(&wav_path);
            return;
        }
    };

    mixer.load_active(wav_path.to_str().unwrap(), 0.0).unwrap();
    mixer.play().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let ev = mixer.poll().unwrap();
    assert!(ev.is_some());

    mixer.stop().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let ev = mixer.poll().unwrap();
    assert!(
        ev.is_none(),
        "poll should return None after stop, got: {ev:?}"
    );

    std::fs::remove_file(&wav_path).ok();
}

#[test]
fn test_mixer_multiple_volume_levels() {
    let wav_path = test_wav_path();
    create_test_wav(&wav_path, 1.0);

    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("no audio device available: {e}, skipping test");
            let _ = std::fs::remove_file(&wav_path);
            return;
        }
    };

    mixer.load_active(wav_path.to_str().unwrap(), 0.0).unwrap();

    for vol in [0, 25, 50, 75, 100] {
        mixer.set_volume(vol).unwrap();
        assert_eq!(mixer.volume(), vol);
    }

    // Clamping
    mixer.set_volume(150).unwrap();
    assert_eq!(mixer.volume(), 100);

    mixer.stop().unwrap();
    std::fs::remove_file(&wav_path).ok();
}

#[test]
fn test_mixer_load_nonexistent_file() {
    let mut mixer = match AudioMixer::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("no audio device available: {e}, skipping test");
            return;
        }
    };

    let result = mixer.load_active("/nonexistent/path/file.wav", 0.0);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("failed to open file"),
        "expected file open error, got: {err}"
    );
}
