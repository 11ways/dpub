use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("epubcheck binary not found on PATH")]
    EpubcheckMissing,

    #[error("ace binary not found on PATH (install: `npm install -g @daisy/ace`)")]
    AceMissing,

    #[error("failed to spawn validator: {0}")]
    Spawn(#[source] std::io::Error),

    #[error("validator exited unexpectedly while checking {path}: {message}")]
    Exited { path: PathBuf, message: String },

    #[error("could not parse validator JSON output: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
