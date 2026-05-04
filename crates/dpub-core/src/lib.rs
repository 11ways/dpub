//! DAISY 2.02 parser and in-memory model.
//!
//! Entry point: [`Book::from_ncc`] reads a DAISY 2.02 publication starting
//! from its `ncc.html` (Navigation Control Centre).

mod error;
mod metadata;
mod ncc;

pub use error::{Error, Result};
pub use metadata::Metadata;
pub use ncc::{Heading, HeadingLevel, NavItem, Ncc, PageNumber};

use std::path::{Path, PathBuf};

/// A parsed DAISY 2.02 publication.
#[derive(Debug)]
pub struct Book {
    pub root: PathBuf,
    pub ncc: Ncc,
}

impl Book {
    /// Load a DAISY 2.02 book from the path to its `ncc.html`.
    pub fn from_ncc(ncc_path: impl AsRef<Path>) -> Result<Self> {
        let ncc_path = ncc_path.as_ref();
        let root = ncc_path
            .parent()
            .ok_or_else(|| Error::InvalidPath(ncc_path.to_path_buf()))?
            .to_path_buf();
        let ncc = Ncc::parse(ncc_path)?;
        Ok(Self { root, ncc })
    }

    /// Convenience accessor for the publication's metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.ncc.metadata
    }
}
