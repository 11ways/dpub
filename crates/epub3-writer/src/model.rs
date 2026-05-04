//! Typed model of an EPUB 3 publication, sized for the audio-with-text-shell
//! profile that DAISY 2.02 audio books map onto cleanly.

use std::path::PathBuf;

/// One complete EPUB 3 publication, ready to be written to a `.epub` ZIP.
///
/// Per-section content, media overlay and audio file lists are kept separate
/// rather than rolled into a single `manifest`/`spine` because in this profile
/// each section produces exactly one content document, one media overlay, and
/// references one or more audio files — and serialisers want them in that
/// shape anyway.
#[derive(Debug, Clone)]
pub struct Publication {
    pub metadata: PackageMetadata,
    pub sections: Vec<SectionPart>,
    pub audio_files: Vec<AudioFile>,
    pub nav: Nav,
}

/// Package-level metadata. Only the EPUB 3-required fields plus the
/// audiobook-relevant accessibility extensions.
#[derive(Debug, Clone, Default)]
pub struct PackageMetadata {
    /// Stable, unique publication identifier (e.g. a UUID URN). Required.
    pub identifier: String,
    /// Publication title. Required.
    pub title: String,
    /// BCP-47 / ISO 639 language code (e.g. `nl`, `en-US`). Required.
    pub language: String,
    /// `dcterms:modified` ISO-8601 timestamp. Required.
    /// If left blank, [`Publication::write_zip`] fills it with the current time.
    pub modified: String,

    pub creator: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub source: Option<String>,
    pub description: Option<String>,

    /// Total media duration of the publication, in seconds.
    pub duration_seconds: Option<f64>,
    pub narrator: Option<String>,

    /// Accessibility access modes — at minimum `Auditory` for audio-only
    /// publications, plus `Textual` if there is real prose in the content
    /// documents (not just heading shells).
    pub access_modes: Vec<AccessMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Auditory,
    Textual,
    Visual,
}

impl AccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessMode::Auditory => "auditory",
            AccessMode::Textual => "textual",
            AccessMode::Visual => "visual",
        }
    }
}

/// A single section: one content XHTML + (optionally) one Media Overlay SMIL.
#[derive(Debug, Clone)]
pub struct SectionPart {
    /// Stable manifest ID (used as `id="..."` in the OPF and `idref` in the spine).
    pub id: String,
    pub content: ContentDocument,
    pub overlay: Option<MediaOverlay>,
}

/// One XHTML content document (typically a thin shell wrapping a heading).
#[derive(Debug, Clone)]
pub struct ContentDocument {
    /// Path inside the EPUB ZIP, relative to the OPF (`EPUB/`). E.g. `content/section-001.xhtml`.
    pub href: String,
    pub title: String,
    pub language: String,
    /// Raw XHTML body content. Trusted: callers are responsible for valid XHTML.
    pub body_xhtml: String,
}

/// One Media Overlay SMIL document.
#[derive(Debug, Clone)]
pub struct MediaOverlay {
    /// Path inside the EPUB ZIP, relative to the OPF (e.g. `media-overlays/section-001.smil`).
    pub href: String,
    /// Total duration, in seconds, of this overlay (sum of all clip durations).
    pub duration_seconds: f64,
    pub root: OverlaySeq,
}

/// EPUB 3 Media Overlays `<seq>`.
#[derive(Debug, Clone, Default)]
pub struct OverlaySeq {
    /// `epub:textref` attribute (typically the content-document href, sometimes anchor).
    pub textref: Option<String>,
    pub children: Vec<OverlayItem>,
}

#[derive(Debug, Clone)]
pub enum OverlayItem {
    Par(OverlayPar),
    Seq(OverlaySeq),
}

/// EPUB 3 Media Overlays `<par>`: pairs one `<text>` with audio.
///
/// The audio half can be a single `<audio>` element or several played in
/// sequence; this profile collapses contiguous fragments of the same audio
/// file into one `OverlayPar` per text-anchor in the content document.
#[derive(Debug, Clone)]
pub struct OverlayPar {
    pub id: Option<String>,
    pub text_src: String,
    pub audio_src: String,
    pub clip_begin_seconds: f64,
    pub clip_end_seconds: f64,
}

/// One audio file in the publication. The bytes come from `source_path` on
/// disk and are streamed into the EPUB ZIP at write time.
#[derive(Debug, Clone)]
pub struct AudioFile {
    /// Stable manifest ID.
    pub id: String,
    /// Path inside the EPUB ZIP, relative to the OPF. E.g. `audio/01.mp3`.
    pub href: String,
    /// Filesystem path to read bytes from at write time.
    pub source_path: PathBuf,
    /// EPUB-recognised media type (`audio/mpeg`, `audio/mp4`, `audio/ogg; codecs=opus`, …).
    pub media_type: String,
}

/// Navigation document content.
#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub toc: Vec<NavListItem>,
    pub page_list: Option<Vec<NavListItem>>,
}

#[derive(Debug, Clone)]
pub struct NavListItem {
    pub label: String,
    pub href: String,
    pub children: Vec<NavListItem>,
}

impl Publication {
    /// Quick sanity check before writing — catches the obvious omissions.
    pub fn validate(&self) -> crate::Result<()> {
        if self.metadata.identifier.is_empty() {
            return Err(crate::Error::InvalidPublication(
                "metadata.identifier empty".into(),
            ));
        }
        if self.metadata.title.is_empty() {
            return Err(crate::Error::InvalidPublication(
                "metadata.title empty".into(),
            ));
        }
        if self.metadata.language.is_empty() {
            return Err(crate::Error::InvalidPublication(
                "metadata.language empty".into(),
            ));
        }
        if self.sections.is_empty() {
            return Err(crate::Error::InvalidPublication("no sections".into()));
        }
        Ok(())
    }
}
