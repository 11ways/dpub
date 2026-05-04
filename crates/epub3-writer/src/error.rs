use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("ZIP I/O error: {0}")]
    ZipIo(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("invalid publication: {0}")]
    InvalidPublication(String),
}

pub type Result<T> = std::result::Result<T, Error>;
