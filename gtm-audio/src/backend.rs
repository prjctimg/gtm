use std::fmt;

pub enum AudioEvent {
    Position(f64),
    Duration(f64),
    Finished,
    Volume(u8),
    Error(String),
}

#[derive(Debug)]
pub enum AudioError {
    OpenFailed(String),
    DecodeError(String),
    OutputError(String),
    UnsupportedFormat(String),
    FfmpegNotFound,
    FfmpegError(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::OpenFailed(msg) => write!(f, "failed to open file: {}", msg),
            AudioError::DecodeError(msg) => write!(f, "decode error: {}", msg),
            AudioError::OutputError(msg) => write!(f, "output error: {}", msg),
            AudioError::UnsupportedFormat(msg) => write!(f, "unsupported format: {}", msg),
            AudioError::FfmpegNotFound => write!(f, "ffmpeg not found"),
            AudioError::FfmpegError(msg) => write!(f, "ffmpeg error: {}", msg),
        }
    }
}

impl std::error::Error for AudioError {}

pub type Result<T> = std::result::Result<T, AudioError>;
