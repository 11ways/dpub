use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("ground truth is empty for section")]
    EmptyGroundTruth,
    #[error("no whisper words for section")]
    NoWhisperWords,
}

pub type Result<T> = std::result::Result<T, Error>;
