use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("epubcheck binary not found on PATH")]
    EpubcheckMissing,

    #[error("failed to spawn epubcheck: {0}")]
    Spawn(#[source] std::io::Error),

    #[error("epubcheck exited unexpectedly while validating {path}: {message}")]
    Exited { path: PathBuf, message: String },

    #[error("could not parse epubcheck JSON output: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
