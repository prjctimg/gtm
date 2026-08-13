use crate::backend::{AudioEvent, Result};

pub struct FfmpegBackend;

impl FfmpegBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn poll(&mut self) -> Result<Option<AudioEvent>> {
        Ok(None)
    }
}
