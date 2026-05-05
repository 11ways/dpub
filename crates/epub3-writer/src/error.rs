use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O failure with the path that caused it.
    ///
    /// Always prefer this variant over a bare `io::Error` so callers see the
    /// path that failed in the error message.
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("invalid publication: {0}")]
    InvalidPublication(String),
}

pub type Result<T> = std::result::Result<T, Error>;
