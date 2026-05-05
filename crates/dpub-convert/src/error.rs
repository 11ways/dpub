use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("DAISY parse error: {0}")]
    Daisy(#[from] dpub_core::Error),

    #[error("EPUB writer error: {0}")]
    Epub(#[from] epub3_writer::Error),

    #[error("audio re-encoding failed: {0}")]
    Audio(#[from] dpub_audio::Error),

    #[error("transcription failed: {0}")]
    Whisper(#[from] dpub_whisper::Error),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
