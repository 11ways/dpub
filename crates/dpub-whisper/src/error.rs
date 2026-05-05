use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model path is not valid UTF-8: {0:?}")]
    InvalidModelPath(PathBuf),

    #[error("failed to load Whisper model from {path}: {source}")]
    ModelLoad {
        path: PathBuf,
        #[source]
        source: whisper_rs::WhisperError,
    },

    #[error("Whisper inference failed: {0}")]
    Whisper(#[from] whisper_rs::WhisperError),

    #[error("audio decode failed for {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: DecodeError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("symphonia error: {0}")]
    Symphonia(#[from] symphonia::core::errors::Error),

    #[error("no audio track in file")]
    NoAudioTrack,

    #[error("resampling failed: {0}")]
    Resample(#[from] rubato::ResampleError),

    #[error("resampler init failed: {0}")]
    ResamplerInit(#[from] rubato::ResamplerConstructionError),
}

pub type Result<T> = std::result::Result<T, Error>;
