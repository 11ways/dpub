//! Build a known-clean minimal EPUB on the fly and run `dpub-validate`
//! against it.
//!
//! Skipped when EPUBCheck is not on `PATH` so the test stays portable.

use std::fs::File;
use std::io::Write;

use epub3_writer::{
    AccessMode, AudioFile, ContentDocument, MediaOverlay, Nav, NavListItem, OverlayItem,
    OverlayPar, OverlaySeq, PackageMetadata, Publication, SectionPart,
};

const TINY_MP3: &[u8] = include_bytes!("../../epub3-writer/tests/fixtures/tiny.mp3");

#[test]
fn validates_a_clean_epub_with_zero_errors() {
    if !dpub_validate::epubcheck_available() {
        eprintln!("epubcheck not on PATH — skipping");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let mp3_path = dir.path().join("tiny.mp3");
    File::create(&mp3_path)
        .and_then(|mut f| f.write_all(TINY_MP3))
        .expect("write mp3");

    let publication = Publication {
        metadata: PackageMetadata {
            identifier: "urn:uuid:11111111-1111-4111-8111-111111111111".into(),
            title: "Validate Me".into(),
            language: "en".into(),
            modified: "2026-05-05T00:00:00Z".into(),
            access_modes: vec![AccessMode::Auditory],
            duration_seconds: Some(0.6),
            ..Default::default()
        },
        nav: Nav {
            toc: vec![NavListItem {
                label: "Validate Me".into(),
                href: "section-001.xhtml".into(),
                children: vec![],
            }],
            page_list: None,
        },
        sections: vec![SectionPart {
            id: "section-001".into(),
            content: ContentDocument {
                href: "section-001.xhtml".into(),
                title: "Validate Me".into(),
                language: "en".into(),
                body_xhtml: r#"<h1 id="h1">Validate Me</h1>"#.into(),
            },
            overlay: Some(MediaOverlay {
                href: "media-overlays/section-001.smil".into(),
                duration_seconds: 0.6,
                root: OverlaySeq {
                    textref: Some("../section-001.xhtml".into()),
                    children: vec![OverlayItem::Par(OverlayPar {
                        id: Some("p1".into()),
                        text_src: "../section-001.xhtml#h1".into(),
                        audio_src: "../audio/tiny.mp3".into(),
                        clip_begin_seconds: 0.0,
                        clip_end_seconds: 0.6,
                    })],
                },
            }),
        }],
        audio_files: vec![AudioFile {
            id: "audio-tiny".into(),
            href: "audio/tiny.mp3".into(),
            source_path: mp3_path,
            media_type: "audio/mpeg".into(),
        }],
        cover: None,
    };

    let epub_path = dir.path().join("clean.epub");
    let mut out = File::create(&epub_path).expect("create epub");
    publication.write_zip(&mut out).expect("write epub");

    let report = dpub_validate::validate_epub(&epub_path).expect("validate");
    let backend = report.epubcheck.as_ref().expect("epubcheck ran");
    assert_eq!(backend.summary.fatals, 0, "issues: {:?}", backend.issues);
    assert_eq!(backend.summary.errors, 0, "issues: {:?}", backend.issues);
    assert!(report.is_clean());
}
