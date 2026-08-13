// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// AudioEvent and AudioError definitions
//
// This is free software released under the GPL-3.0 license.

pub type AudioResult<T> = std::result::Result<T, AudioError>;

#[derive(Debug, Clone)]
pub enum AudioEvent {
    Position(f64),
    Duration(f64),
    Finished,
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to open file: {0}")]
    OpenFailed(String),
    #[error("decode error: {0}")]
    DecodeError(String),
    #[error("output error: {0}")]
    OutputError(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("seek error: {0}")]
    SeekError(String),
}

impl From<AudioError> for gtm_core::CoreError {
    fn from(e: AudioError) -> Self {
        gtm_core::CoreError::Daemon(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_error_display() {
        assert_eq!(
            AudioError::OpenFailed("foo".into()).to_string(),
            "failed to open file: foo"
        );
        assert_eq!(
            AudioError::DecodeError("bar".into()).to_string(),
            "decode error: bar"
        );
        assert_eq!(
            AudioError::OutputError("baz".into()).to_string(),
            "output error: baz"
        );
        assert_eq!(
            AudioError::UnsupportedFormat("aac".into()).to_string(),
            "unsupported format: aac"
        );
        assert_eq!(
            AudioError::SeekError("oops".into()).to_string(),
            "seek error: oops"
        );
    }

    #[test]
    fn test_audio_error_from_to_core() {
        let err = AudioError::OpenFailed("x".into());
        let core: gtm_core::CoreError = err.into();
        assert_eq!(core.to_string(), "daemon error: failed to open file: x");
    }

    #[test]
    fn test_audio_event_debug_clone() {
        let ev = AudioEvent::Position(42.5);
        let cloned = ev.clone();
        assert_eq!(format!("{:?}", cloned), "Position(42.5)");

        let ev = AudioEvent::Duration(180.0);
        let cloned = ev.clone();
        assert_eq!(format!("{:?}", cloned), "Duration(180.0)");

        let ev = AudioEvent::Finished;
        let cloned = ev.clone();
        assert_eq!(format!("{:?}", cloned), "Finished");

        let ev = AudioEvent::Error("msg".into());
        let cloned = ev.clone();
        assert_eq!(format!("{:?}", cloned), "Error(\"msg\")");
    }
}
