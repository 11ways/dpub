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
    /// Optional book cover image. Surfaced in the OPF manifest as
    /// `<item ... properties="cover-image">` per EPUB 3.3 §5.5.4.
    pub cover: Option<CoverImage>,
}

/// One book cover image, embedded as a manifest item with the
/// `cover-image` property. Bytes are held in memory because covers are
/// small (typically <1 MiB) and held only while the [`Publication`] is
/// being written.
#[derive(Debug, Clone)]
pub struct CoverImage {
    /// Path inside the EPUB ZIP, relative to the OPF (`EPUB/`). E.g.
    /// `images/cover.jpg`.
    pub href: String,
    /// `image/jpeg` or `image/png`. The writer accepts any IANA image
    /// media type, but typical covers are one of these two.
    pub media_type: String,
    /// Raw image bytes, written verbatim into the ZIP entry.
    pub bytes: Vec<u8>,
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
    /// Total duration of this overlay in seconds (sum of every audio clip
    /// span). Surfaced in the OPF as a `<meta property="media:duration"
    /// refines="#…-overlay">` element — required by EPUB 3 when the overlay
    /// is referenced from a manifest item.
    pub duration_seconds: f64,
    /// Root `<seq>` of the overlay. Children are usually flat lists of
    /// `<par>`s (one per text-anchor sync point), but nested `<seq>`s are
    /// allowed to mirror the SMIL grammar.
    pub root: OverlaySeq,
}

/// EPUB 3 Media Overlays `<seq>`.
#[derive(Debug, Clone, Default)]
pub struct OverlaySeq {
    /// Value emitted as the `epub:textref` attribute. Must be a path
    /// relative to *the SMIL file's location* (so for a SMIL at
    /// `EPUB/media-overlays/foo.smil` pointing at `EPUB/foo.xhtml`, the
    /// value is `../foo.xhtml`). EPUBCheck will fail if this is wrong.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> Publication {
        Publication {
            metadata: PackageMetadata {
                identifier: "urn:uuid:0".into(),
                title: "T".into(),
                language: "en".into(),
                modified: String::new(),
                ..Default::default()
            },
            nav: Nav::default(),
            sections: vec![SectionPart {
                id: "s1".into(),
                content: ContentDocument {
                    href: "s1.xhtml".into(),
                    title: "T".into(),
                    language: "en".into(),
                    body_xhtml: String::new(),
                },
                overlay: None,
            }],
            audio_files: vec![],
            cover: None,
        }
    }

    #[test]
    fn validate_accepts_minimal_publication() {
        assert!(minimal().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_identifier() {
        let mut p = minimal();
        p.metadata.identifier.clear();
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("identifier"), "{err}");
    }

    #[test]
    fn validate_rejects_empty_title() {
        let mut p = minimal();
        p.metadata.title.clear();
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("title"), "{err}");
    }

    #[test]
    fn validate_rejects_empty_language() {
        let mut p = minimal();
        p.metadata.language.clear();
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("language"), "{err}");
    }

    #[test]
    fn validate_rejects_no_sections() {
        let mut p = minimal();
        p.sections.clear();
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("section"), "{err}");
    }
}
