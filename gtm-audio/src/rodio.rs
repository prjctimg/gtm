use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

use crate::backend::{AudioBackend, AudioError, AudioEvent, AudioResult};

pub struct RodioBackend {
    _sink: Arc<MixerDeviceSinkWrapper>,
    player: Player,
    position: Arc<Mutex<f64>>,
    duration: Arc<Mutex<f64>>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
    start_time: Arc<Mutex<Option<Instant>>>,
    start_pos: Arc<Mutex<f64>>,
}

struct MixerDeviceSinkWrapper(rodio::MixerDeviceSink);

impl RodioBackend {
    pub fn new() -> AudioResult<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| AudioError::OutputError(e.to_string()))?;
        sink.log_on_drop(false);
        let sink = Arc::new(MixerDeviceSinkWrapper(sink));
        let player = Player::connect_new(sink.0.mixer());
        Ok(Self {
            _sink: sink,
            player,
            position: Arc::new(Mutex::new(0.0)),
            duration: Arc::new(Mutex::new(0.0)),
            playing: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU8::new(100)),
            start_time: Arc::new(Mutex::new(None)),
            start_pos: Arc::new(Mutex::new(0.0)),
        })
    }
}

#[async_trait]
impl AudioBackend for RodioBackend {
    async fn load(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        let file = File::open(path).map_err(|e| AudioError::OpenFailed(e.to_string()))?;
        let reader = BufReader::new(file);
        let source = Decoder::new(reader).map_err(|e| AudioError::DecodeError(e.to_string()))?;

        let total = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        *self.duration.lock().unwrap() = total;
        *self.position.lock().unwrap() = start_pos;

        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        self.player.set_volume(vol);
        self.player.append(source);

        if start_pos > 0.0 {
            let pos = std::time::Duration::from_secs_f64(start_pos);
            let _ = self.player.try_seek(pos);
        }

        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);

        Ok(())
    }

    async fn play(&mut self) -> AudioResult<()> {
        if self.player.is_paused() {
            self.player.play();
        }
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn pause(&mut self) -> AudioResult<()> {
        if !self.player.is_paused() {
            self.player.pause();
        }
        let elapsed = self
            .start_time
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let current = *self.start_pos.lock().unwrap();
        *self.position.lock().unwrap() = current + elapsed;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&mut self) -> AudioResult<()> {
        self.player.stop();
        *self.position.lock().unwrap() = 0.0;
        self.playing.store(false, Ordering::SeqCst);
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = 0.0;
        Ok(())
    }

    async fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        let pos = std::time::Duration::from_secs_f64(position_secs);
        self.player
            .try_seek(pos)
            .map_err(|_| AudioError::SeekError("seek failed".into()))?;
        *self.position.lock().unwrap() = position_secs;
        *self.start_time.lock().unwrap() = (position_secs > 0.0).then(|| Instant::now());
        *self.start_pos.lock().unwrap() = position_secs;
        Ok(())
    }

    async fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        let vol = volume.min(100) as f32 / 100.0;
        self.volume.store(volume.min(100), Ordering::SeqCst);
        self.player.set_volume(vol);
        Ok(())
    }

    async fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        if self.player.empty() {
            if self.playing.load(Ordering::SeqCst) {
                self.playing.store(false, Ordering::SeqCst);
                return Ok(Some(AudioEvent::Finished));
            }
            return Ok(None);
        }

        if !self.player.is_paused() {
            let elapsed = self
                .start_time
                .lock()
                .unwrap()
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            let start = *self.start_pos.lock().unwrap();
            let total = *self.duration.lock().unwrap();
            let pos = (start + elapsed).min(total);
            *self.position.lock().unwrap() = pos;
            return Ok(Some(AudioEvent::Position(pos)));
        }

        Ok(None)
    }

    fn current_position(&self) -> f64 {
        *self.position.lock().unwrap()
    }

    fn duration(&self) -> f64 {
        *self.duration.lock().unwrap()
    }

    fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    fn volume(&self) -> u8 {
        self.volume.load(Ordering::SeqCst)
    }
}
