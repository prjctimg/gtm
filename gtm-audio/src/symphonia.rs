use crate::backend::{AudioEvent, Result};

pub struct SymphoniaBackend;

impl SymphoniaBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn poll(&mut self) -> Result<Option<AudioEvent>> {
        Ok(None)
    }
}
