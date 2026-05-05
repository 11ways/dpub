#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(#[from] Box<ureq::Error>),

    #[error("malformed JSON from Open Library: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Open Library returned a placeholder, not a real cover")]
    NoCover,

    #[error("cover bytes are not a recognised image format")]
    UnsupportedFormat,
}

// `ureq::Error` is large; box it to keep `Result<_, Error>` cheap.
impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Network(Box::new(e))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
