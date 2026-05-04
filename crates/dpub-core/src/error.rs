use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid DAISY path: {0}")]
    InvalidPath(PathBuf),

    #[error("I/O error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("XML parse error in {path}: {source}")]
    Xml {
        path: PathBuf,
        #[source]
        source: quick_xml::Error,
    },

    #[error("malformed NCC at {path}: {message}")]
    MalformedNcc { path: PathBuf, message: String },
}

pub type Result<T> = std::result::Result<T, Error>;
