// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Symphonia-based audio decoder for seeking and format detection
//
// This is free software released under the GPL-3.0 license.

use std::fs::File;
use std::io::{Read, SeekFrom};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Mutex;
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

use crate::backend::{AudioError, AudioResult};

fn build_codec_registry() -> CodecRegistry {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
}

fn build_probe() -> Probe {
    let mut probe = Probe::new();
    symphonia::default::register_enabled_formats(&mut probe);
    probe
}

/// Re-opens a streaming byte source from scratch. Used for seeking: a seek
/// re-issues the transport (reconnect / Range request) and rebuilds the probe
/// + decoder over the fresh reader.
pub trait StreamingReopen: Send {
    /// Produce a fresh reader positioned at the source start.
    fn try_reopen(&self) -> Option<Box<dyn Read + Send>>;
}

/// Local-file re-opener retained for `SymphoniaSource`'s original seek path.
struct FileReopen {
    path: String,
}

impl StreamingReopen for FileReopen {
    fn try_reopen(&self) -> Option<Box<dyn Read + Send>> {
        File::open(&self.path)
            .map(|f| Box::new(f) as Box<dyn Read + Send>)
            .ok()
    }
}

/// Wraps a boxed reader in interior mutability so a `Read + Send` stream also
/// satisfies symphonia's `Send + Sync` `MediaSource` bound.
struct SyncReader(Mutex<Box<dyn Read + Send>>);

impl Read for SyncReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

impl std::io::Seek for SyncReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::other("streaming source does not support seeking"))
    }
}

/// A `rodio::Source` backed by symphonia's own decoder pipeline.
pub struct SymphoniaSource {
    reader: Box<dyn FormatReader>,
    track_id: u32,
    codec_params: symphonia::core::codecs::audio::AudioCodecParameters,
    opts: AudioDecoderOptions,
    decoder: Box<dyn AudioDecoder>,
    buffer: Vec<f32>,
    buffer_pos: usize,
    channels: NonZeroU16,
    sample_rate: NonZeroU32,
    eof: bool,
    duration: f64,
    /// Optional re-opener used by `try_seek`. `None` for one-shot sources
    /// that cannot be reconnected.
    reopen: Option<Box<dyn StreamingReopen>>,
    /// Number of leading samples to skip before producing output.
    seek_skip: u64,
}

impl SymphoniaSource {
    pub fn from_file(
        path: &str,
        start_pos: f64,
    ) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        let file = File::open(path).map_err(|e| AudioError::OpenFailed(e.to_string()))?;
        let reopen: Box<dyn StreamingReopen> = Box::new(FileReopen {
            path: path.to_string(),
        });
        Self::from_reader(file, Some(reopen), start_pos)
            .map(|s| Box::new(s) as Box<dyn Source<Item = f32> + Send>)
    }

    /// Build a symphonia pipeline over an arbitrary byte stream. `reopen`
    /// enables seeking by reconnecting; `None` makes seek report
    /// `NotSupported` (appropriate for live, non-seekable transports).
    pub fn from_reader(
        reader: impl Read + Send + 'static,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<Self> {
        let mss = MediaSourceStream::new(
            Box::new(ReadOnlySource::new(SyncReader(Mutex::new(
                Box::new(reader) as Box<dyn Read + Send>
            )))),
            MediaSourceStreamOptions::default(),
        );
        let probe = build_probe();
        let hint = Hint::new();
        let fmt_opts = FormatOptions::default();
        let meta_opts = MetadataOptions::default();
        let reader = probe
            .probe(&hint, mss, fmt_opts, meta_opts)
            .map_err(|e| AudioError::DecodeError(format!("probe failed: {e}")))?;

        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::UnsupportedFormat("no audio track found".into()))?;

        let codec_params = track
            .codec_params
            .as_ref()
            .and_then(|p| match p {
                CodecParameters::Audio(a) => Some(a.clone()),
                _ => None,
            })
            .ok_or_else(|| AudioError::UnsupportedFormat("no audio codec params".into()))?;

        let duration = track
            .duration
            .zip(track.time_base)
            .and_then(|(d, tb)| {
                let ts = Timestamp::new(d.get() as i64);
                tb.calc_time(ts).map(|t| t.as_secs_f64())
            })
            .unwrap_or(0.0);
        let track_id = track.id;
        let opts = AudioDecoderOptions::default();

        let seek_skip = if start_pos > 0.0 {
            let sample_rate = codec_params.sample_rate.unwrap_or(44100);
            let ch = codec_params
                .channels
                .as_ref()
                .map(|c| c.count() as u64)
                .unwrap_or(2);
            (start_pos * sample_rate as f64 * ch as f64) as u64
        } else {
            0
        };

        let s = Self {
            reader,
            track_id,
            codec_params: codec_params.clone(),
            opts,
            decoder: build_codec_registry()
                .make_audio_decoder(&codec_params, &opts)
                .map_err(|e| AudioError::DecodeError(format!("decoder init failed: {e}")))?,
            buffer: Vec::new(),
            buffer_pos: 0,
            channels: NonZeroU16::new(
                codec_params
                    .channels
                    .as_ref()
                    .map(|c| c.count() as u16)
                    .unwrap_or(2),
            )
            .unwrap_or(NonZeroU16::MIN),
            sample_rate: NonZeroU32::new(codec_params.sample_rate.unwrap_or(44100))
                .unwrap_or(NonZeroU32::MIN),
            eof: false,
            duration,
            reopen,
            seek_skip,
        };
        Ok(s)
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.buffer_pos < self.buffer.len() {
                let sample = self.buffer[self.buffer_pos];
                self.buffer_pos += 1;

                // If we are still in a seek-skip phase, discard samples
                if self.seek_skip > 0 {
                    self.seek_skip -= 1;
                    continue;
                }

                return Some(sample);
            }
            if self.eof {
                return None;
            }
            self.buffer.clear();
            self.buffer_pos = 0;
            loop {
                match self.reader.next_packet() {
                    Ok(Some(packet)) => {
                        if packet.track_id != self.track_id {
                            continue;
                        }
                        match self.decoder.decode(&packet) {
                            Ok(buf) => {
                                buf.copy_to_vec_interleaved::<f32>(&mut self.buffer);

                                // If seeking, skip entire decoded buffer in one go when possible
                                if self.seek_skip > 0 && !self.buffer.is_empty() {
                                    let skip = self.seek_skip.min(self.buffer.len() as u64);
                                    self.buffer_pos = skip as usize;
                                    self.seek_skip -= skip;
                                }

                                break;
                            }
                            Err(symphonia::core::errors::Error::DecodeError(d)) => {
                                log::warn!("decode error: {d}");
                                continue;
                            }
                            Err(symphonia::core::errors::Error::IoError(e)) => {
                                log::error!("io error: {e}");
                                self.eof = true;
                                return None;
                            }
                            Err(symphonia::core::errors::Error::ResetRequired) => {
                                self.reader
                                    .tracks()
                                    .iter()
                                    .find(|t| t.id == self.track_id)
                                    .and_then(|t| t.codec_params.as_ref())
                                    .and_then(|p| match p {
                                        CodecParameters::Audio(a) => {
                                            self.codec_params = a.clone();
                                            let registry = build_codec_registry();
                                            match registry
                                                .make_audio_decoder(&a.clone(), &self.opts)
                                            {
                                                Ok(d) => {
                                                    self.decoder = d;
                                                    Some(())
                                                }
                                                Err(_) => None,
                                            }
                                        }
                                        _ => None,
                                    });
                                continue;
                            }
                            Err(e) => {
                                log::error!("unrecoverable error: {e}");
                                self.eof = true;
                                return None;
                            }
                        }
                    }
                    Ok(None) => {
                        self.eof = true;
                        return None;
                    }
                    Err(e) => {
                        log::error!("packet read error: {e}");
                        self.eof = true;
                        return None;
                    }
                }
            }
        }
    }
}

impl Source for SymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.duration > 0.0 {
            Some(Duration::from_secs_f64(self.duration))
        } else {
            None
        }
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let target_secs = pos.as_secs_f64();
        if target_secs < 0.0 || (self.duration > 0.0 && target_secs > self.duration) {
            return Err(SeekError::NotSupported {
                underlying_source: std::any::type_name::<Self>(),
            });
        }

        // Re-open the transport and re-initialize the reader + decoder.
        let fresh: Box<dyn Read + Send> = match self.reopen.as_ref() {
            Some(reopen) => reopen.try_reopen().ok_or(SeekError::NotSupported {
                underlying_source: std::any::type_name::<Self>(),
            })?,
            None => {
                return Err(SeekError::NotSupported {
                    underlying_source: std::any::type_name::<Self>(),
                });
            }
        };
        let mss = MediaSourceStream::new(
            Box::new(ReadOnlySource::new(SyncReader(Mutex::new(fresh)))),
            MediaSourceStreamOptions::default(),
        );
        let probe = build_probe();
        let hint = Hint::new();
        let fmt_opts = FormatOptions::default();
        let meta_opts = MetadataOptions::default();
        let reader =
            probe
                .probe(&hint, mss, fmt_opts, meta_opts)
                .map_err(|_| SeekError::NotSupported {
                    underlying_source: std::any::type_name::<Self>(),
                })?;
        let track =
            reader
                .default_track(TrackType::Audio)
                .ok_or_else(|| SeekError::NotSupported {
                    underlying_source: std::any::type_name::<Self>(),
                })?;
        let codec_params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(a)) => a.clone(),
            _ => {
                return Err(SeekError::NotSupported {
                    underlying_source: std::any::type_name::<Self>(),
                });
            }
        };
        let track_id = track.id;
        let registry = build_codec_registry();
        let decoder = registry
            .make_audio_decoder(&codec_params, &self.opts)
            .map_err(|_| SeekError::NotSupported {
                underlying_source: std::any::type_name::<Self>(),
            })?;

        let target_samples =
            (target_secs * self.sample_rate.get() as f64 * self.channels.get() as f64) as u64;

        self.reader = reader;
        self.track_id = track_id;
        self.codec_params = codec_params;
        self.decoder = decoder;
        self.buffer.clear();
        self.buffer_pos = 0;
        self.eof = false;
        self.seek_skip = target_samples;

        Ok(())
    }
}
