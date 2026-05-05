//! DAISY 2.02 parser and in-memory model.
//!
//! Entry point: [`Book::from_ncc`] reads a DAISY 2.02 publication starting
//! from its `ncc.html` (Navigation Control Centre).

mod error;
mod metadata;
mod ncc;
pub mod smil;
mod smil_write;
pub mod time;

pub use smil_write::{write_master_smil, write_section_smil};

pub use error::{Error, Result};
pub use metadata::Metadata;
pub use ncc::{Heading, HeadingLevel, NavItem, Ncc, PageNumber};
pub use smil::{
    AudioClip, MasterSmil, ParChild, SectionRef, SectionSmil, SeqChild, SmilMetadata, SmilPar,
    SmilSeq, TextRef,
};

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A parsed DAISY 2.02 publication.
#[derive(Debug)]
pub struct Book {
    /// Filesystem directory the publication was loaded from. Used by
    /// downstream crates (notably [`dpub-convert`]) to resolve audio file
    /// references inside SMIL clips back to bytes on disk.
    pub root: PathBuf,
    /// Parsed `ncc.html` — the navigation control centre.
    pub ncc: Ncc,
    /// Parsed `master.smil` — the publication's spine of section refs.
    pub master: MasterSmil,
    /// Parsed per-section SMIL files, in `master.references` order.
    pub sections: Vec<SectionSmil>,
}

impl Book {
    /// Load a full DAISY 2.02 book from the path to its `ncc.html`.
    ///
    /// Resolves and parses `master.smil` and every per-section SMIL file
    /// referenced from it (relative to the NCC's directory).
    pub fn from_ncc(ncc_path: impl AsRef<Path>) -> Result<Self> {
        let ncc_path = ncc_path.as_ref();
        let root = ncc_path
            .parent()
            .ok_or_else(|| Error::InvalidPath(ncc_path.to_path_buf()))?
            .to_path_buf();

        let ncc = Ncc::parse(ncc_path)?;
        let master = MasterSmil::parse(root.join("master.smil"))?;
        let sections = master
            .references
            .iter()
            .map(|r| SectionSmil::parse(root.join(&r.src)))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            root,
            ncc,
            master,
            sections,
        })
    }

    /// Convenience accessor for the NCC's metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.ncc.metadata
    }

    /// Aggregate audio duration across every clip in every section, in seconds.
    pub fn total_audio_seconds(&self) -> f64 {
        self.sections
            .iter()
            .flat_map(audio_clips)
            .map(AudioClip::duration)
            .sum()
    }

    /// Number of `<par>` elements across all sections (synchronisation points).
    pub fn total_par_count(&self) -> usize {
        self.sections
            .iter()
            .map(|s| count_pars_in_seq(&s.root))
            .sum()
    }

    /// Number of audio clips across all sections.
    pub fn total_audio_clip_count(&self) -> usize {
        self.sections.iter().map(|s| audio_clips(s).count()).sum()
    }

    /// Distinct audio source filenames referenced anywhere in the book,
    /// in their first-seen order.
    pub fn audio_files(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for section in &self.sections {
            for clip in audio_clips(section) {
                if seen.insert(clip.src.clone()) {
                    ordered.push(clip.src.clone());
                }
            }
        }
        ordered
    }
}

/// Iterate every [`AudioClip`] inside a single section, regardless of how
/// deeply it is nested in `<seq>` / `<par>` containers.
pub fn audio_clips(section: &SectionSmil) -> Box<dyn Iterator<Item = &AudioClip> + '_> {
    Box::new(seq_audio(&section.root))
}

fn seq_audio(seq: &SmilSeq) -> Box<dyn Iterator<Item = &AudioClip> + '_> {
    Box::new(
        seq.children
            .iter()
            .flat_map(|child| -> Box<dyn Iterator<Item = &AudioClip>> {
                match child {
                    SeqChild::Seq(s) => seq_audio(s),
                    SeqChild::Par(p) => par_audio(p),
                    SeqChild::Audio(a) => Box::new(std::iter::once(a)),
                }
            }),
    )
}

fn par_audio(par: &SmilPar) -> Box<dyn Iterator<Item = &AudioClip> + '_> {
    Box::new(
        par.children
            .iter()
            .flat_map(|child| -> Box<dyn Iterator<Item = &AudioClip>> {
                match child {
                    ParChild::Audio(a) => Box::new(std::iter::once(a)),
                    ParChild::Seq(s) => seq_audio(s),
                    ParChild::Text(_) => Box::new(std::iter::empty()),
                }
            }),
    )
}

fn count_pars_in_seq(seq: &SmilSeq) -> usize {
    seq.children
        .iter()
        .map(|c| match c {
            SeqChild::Par(p) => 1 + count_pars_in_par(p),
            SeqChild::Seq(s) => count_pars_in_seq(s),
            SeqChild::Audio(_) => 0,
        })
        .sum()
}

fn count_pars_in_par(par: &SmilPar) -> usize {
    par.children
        .iter()
        .map(|c| match c {
            ParChild::Seq(s) => count_pars_in_seq(s),
            ParChild::Audio(_) | ParChild::Text(_) => 0,
        })
        .sum()
}
