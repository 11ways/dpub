//! Build a minimal audio-only EPUB 3 publication and assert that:
//!
//! - The ZIP archive is structurally well-formed (mimetype is the first
//!   entry, stored uncompressed, exactly the expected bytes — checked by
//!   reading the raw archive ourselves).
//! - If `epubcheck` is available on `PATH`, the publication validates with
//!   zero EPUB errors. Skipped on machines without `epubcheck`.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use epub3_writer::{
    AccessMode, AudioFile, ContentDocument, CoverImage, MediaOverlay, Nav, NavListItem,
    OverlayItem, OverlayPar, OverlaySeq, PackageMetadata, Publication, SectionPart,
};

/// 1.6 KiB of MP3 silence (constant-bitrate, 44.1 kHz mono). Just enough for
/// EPUB readers and validators to accept the publication without complaining
/// about empty audio. Generated separately and embedded as a byte literal.
///
/// Decoded duration: roughly 0.04 s per frame × 16 frames ≈ 0.6 s.
const TINY_MP3: &[u8] = include_bytes!("fixtures/tiny.mp3");

fn build_minimal_pub(audio_path: &Path) -> Publication {
    let title = "Tiny Talking Book";

    Publication {
        metadata: PackageMetadata {
            identifier: "urn:uuid:00000000-0000-4000-8000-000000000001".into(),
            title: title.into(),
            language: "en".into(),
            modified: "2026-05-05T00:00:00Z".into(),
            creator: Some("Eleven Ways".into()),
            duration_seconds: Some(0.6),
            narrator: Some("Robotic Voice".into()),
            access_modes: vec![AccessMode::Auditory],
            ..Default::default()
        },
        nav: Nav {
            toc: vec![NavListItem {
                label: title.into(),
                href: "section-001.xhtml".into(),
                children: vec![],
            }],
            page_list: None,
        },
        sections: vec![SectionPart {
            id: "section-001".into(),
            content: ContentDocument {
                href: "section-001.xhtml".into(),
                title: title.into(),
                language: "en".into(),
                body_xhtml: r#"<h1 id="h1">Tiny Talking Book</h1>"#.into(),
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
            source_path: audio_path.to_path_buf(),
            media_type: "audio/mpeg".into(),
        }],
        cover: None,
    }
}

/// 67-byte 1x1 transparent PNG, the smallest valid PNG file.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn write_publication_to(dir: &Path) -> std::path::PathBuf {
    let audio_path = dir.join("tiny.mp3");
    let mut f = File::create(&audio_path).expect("create audio fixture");
    f.write_all(TINY_MP3).expect("write audio fixture");

    let publication = build_minimal_pub(&audio_path);
    let epub_path = dir.join("minimal.epub");
    let mut out = File::create(&epub_path).expect("create epub file");
    publication.write_zip(&mut out).expect("write epub");
    epub_path
}

#[test]
fn ocf_first_entry_is_uncompressed_mimetype() {
    let dir = tempfile::tempdir().expect("tempdir");
    let epub_path = write_publication_to(dir.path());

    let bytes = std::fs::read(&epub_path).expect("read epub");

    // Local file header: 4-byte signature, then 26 bytes of header fields.
    // We assert the layout by hand rather than re-using a ZIP library so
    // bugs in the ZIP library don't mask bugs in our writer.
    assert_eq!(
        &bytes[0..4],
        b"PK\x03\x04",
        "missing local-file-header signature"
    );

    let compression_method = u16::from_le_bytes([bytes[8], bytes[9]]);
    assert_eq!(
        compression_method, 0,
        "mimetype must be Stored, got method {compression_method}"
    );

    let filename_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
    assert_eq!(extra_len, 0, "mimetype must have no extra field");

    let filename = std::str::from_utf8(&bytes[30..30 + filename_len]).expect("ascii filename");
    assert_eq!(filename, "mimetype");

    let body_offset = 30 + filename_len + extra_len;
    let body = &bytes[body_offset..body_offset + b"application/epub+zip".len()];
    assert_eq!(body, b"application/epub+zip");
}

#[test]
fn epub_archive_contains_required_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let epub_path = write_publication_to(dir.path());

    let f = File::open(&epub_path).expect("open");
    let mut archive = zip::ZipArchive::new(f).expect("zip");
    let names: std::collections::BTreeSet<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_owned())
        .collect();

    let must_have = [
        "mimetype",
        "META-INF/container.xml",
        "EPUB/package.opf",
        "EPUB/nav.xhtml",
        "EPUB/section-001.xhtml",
        "EPUB/media-overlays/section-001.smil",
        "EPUB/audio/tiny.mp3",
    ];
    for required in must_have {
        assert!(
            names.contains(required),
            "missing entry: {required} (have {names:?})"
        );
    }

    // Spot-check that the OPF referenced from container.xml actually exists.
    let mut container = archive
        .by_name("META-INF/container.xml")
        .expect("container");
    let mut s = String::new();
    container.read_to_string(&mut s).unwrap();
    assert!(s.contains("EPUB/package.opf"));
}

/// Run epubcheck against the produced publication if it is available. This
/// catches structural issues quick-eye review can miss (missing required
/// metadata, invalid SMIL, broken cross-references, …).
#[test]
fn epubcheck_clean_when_available() {
    let Ok(epubcheck) = which("epubcheck") else {
        eprintln!("epubcheck not on PATH — skipping");
        return;
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let epub_path = write_publication_to(dir.path());

    let output = Command::new(epubcheck)
        .arg(&epub_path)
        .output()
        .expect("run epubcheck");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        output.status.success(),
        "epubcheck reported errors:\n{combined}",
    );
    // Even a "successful" epubcheck run can emit warnings; our minimal book
    // shouldn't produce any. Fail loudly if it ever does — easier to keep
    // output clean than to debug warnings six months from now.
    assert!(
        !combined.contains("WARNING"),
        "epubcheck emitted warnings:\n{combined}",
    );
}

#[test]
fn cover_image_is_embedded_and_referenced() {
    let dir = tempfile::tempdir().expect("tempdir");

    let audio_path = dir.path().join("tiny.mp3");
    std::fs::write(&audio_path, TINY_MP3).expect("write audio fixture");

    let mut publication = build_minimal_pub(&audio_path);
    publication.cover = Some(CoverImage {
        href: "images/cover.png".into(),
        media_type: "image/png".into(),
        bytes: TINY_PNG.to_vec(),
    });

    let epub_path = dir.path().join("with-cover.epub");
    let mut out = File::create(&epub_path).expect("create");
    publication.write_zip(&mut out).expect("write");

    let f = File::open(&epub_path).expect("open");
    let mut archive = zip::ZipArchive::new(f).expect("zip");

    // Cover bytes are present in the ZIP at the expected path.
    let mut cover_bytes = Vec::new();
    archive
        .by_name("EPUB/images/cover.png")
        .expect("cover entry missing")
        .read_to_end(&mut cover_bytes)
        .expect("read cover");
    assert_eq!(cover_bytes, TINY_PNG);

    // OPF references the cover with `properties="cover-image"`.
    let mut opf = String::new();
    archive
        .by_name("EPUB/package.opf")
        .expect("opf")
        .read_to_string(&mut opf)
        .expect("read opf");
    assert!(
        opf.contains("properties=\"cover-image\""),
        "OPF missing cover-image property:\n{opf}"
    );
    assert!(
        opf.contains("href=\"images/cover.png\""),
        "OPF missing cover href:\n{opf}"
    );
    assert!(
        opf.contains("media-type=\"image/png\""),
        "OPF missing cover media-type:\n{opf}"
    );
}

#[test]
fn epubcheck_clean_with_cover_when_available() {
    let Ok(epubcheck) = which("epubcheck") else {
        eprintln!("epubcheck not on PATH — skipping");
        return;
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let audio_path = dir.path().join("tiny.mp3");
    std::fs::write(&audio_path, TINY_MP3).expect("write audio fixture");

    let mut publication = build_minimal_pub(&audio_path);
    publication.cover = Some(CoverImage {
        href: "images/cover.png".into(),
        media_type: "image/png".into(),
        bytes: TINY_PNG.to_vec(),
    });

    let epub_path = dir.path().join("with-cover.epub");
    let mut out = File::create(&epub_path).expect("create");
    publication.write_zip(&mut out).expect("write");

    let output = Command::new(epubcheck)
        .arg(&epub_path)
        .output()
        .expect("run epubcheck");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.status.success(),
        "epubcheck reported errors:\n{combined}",
    );
    assert!(
        !combined.contains("WARNING"),
        "epubcheck emitted warnings:\n{combined}",
    );
}

/// Build a minimal publication whose Media Overlay uses **word-level**
/// sync — nested `<seq epub:textref="...#tx-...">` per paragraph
/// wrapping per-word `<par>` entries. Mirrors what `dpub-convert`
/// produces with M6.5 word-level sync enabled. Asserts EPUBCheck stays
/// clean for the new structure.
#[test]
fn epubcheck_clean_with_word_level_overlay_when_available() {
    let Ok(epubcheck) = which("epubcheck") else {
        eprintln!("epubcheck not on PATH — skipping");
        return;
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let audio_path = dir.path().join("tiny.mp3");
    std::fs::write(&audio_path, TINY_MP3).expect("write audio fixture");

    let title = "Word-level sync";
    // Two-word paragraph anchored as `tx-000-000`. The XHTML body
    // wraps each word in a `<span id="w-...">` so the overlay's
    // `<text src="...#w-...">` resolves.
    let body = r#"<h1 id="h1">Word-level sync</h1>
  <p id="tx-000-000"><span id="w-000-000-000">Hallo</span> <span id="w-000-000-001">wereld.</span></p>"#;
    let publication = Publication {
        metadata: PackageMetadata {
            identifier: "urn:uuid:00000000-0000-4000-8000-000000000002".into(),
            title: title.into(),
            language: "nl".into(),
            modified: "2026-05-06T00:00:00Z".into(),
            duration_seconds: Some(0.6),
            access_modes: vec![AccessMode::Auditory, AccessMode::Textual],
            ..Default::default()
        },
        nav: Nav {
            toc: vec![NavListItem {
                label: title.into(),
                href: "section-001.xhtml".into(),
                children: vec![],
            }],
            page_list: None,
        },
        sections: vec![SectionPart {
            id: "section-001".into(),
            content: ContentDocument {
                href: "section-001.xhtml".into(),
                title: title.into(),
                language: "nl".into(),
                body_xhtml: body.into(),
            },
            overlay: Some(MediaOverlay {
                href: "media-overlays/section-001.smil".into(),
                duration_seconds: 0.6,
                root: OverlaySeq {
                    textref: Some("../section-001.xhtml".into()),
                    children: vec![OverlayItem::Seq(OverlaySeq {
                        textref: Some("../section-001.xhtml#tx-000-000".into()),
                        children: vec![
                            OverlayItem::Par(OverlayPar {
                                id: Some("w-000-000-000".into()),
                                text_src: "../section-001.xhtml#w-000-000-000".into(),
                                audio_src: "../audio/tiny.mp3".into(),
                                clip_begin_seconds: 0.0,
                                clip_end_seconds: 0.3,
                            }),
                            OverlayItem::Par(OverlayPar {
                                id: Some("w-000-000-001".into()),
                                text_src: "../section-001.xhtml#w-000-000-001".into(),
                                audio_src: "../audio/tiny.mp3".into(),
                                clip_begin_seconds: 0.3,
                                clip_end_seconds: 0.6,
                            }),
                        ],
                    })],
                },
            }),
        }],
        audio_files: vec![AudioFile {
            id: "audio-tiny".into(),
            href: "audio/tiny.mp3".into(),
            source_path: audio_path.clone(),
            media_type: "audio/mpeg".into(),
        }],
        cover: None,
    };

    let epub_path = dir.path().join("word-sync.epub");
    let mut out = File::create(&epub_path).expect("create");
    publication.write_zip(&mut out).expect("write");

    let output = Command::new(epubcheck)
        .arg(&epub_path)
        .output()
        .expect("run epubcheck");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.status.success(),
        "epubcheck reported errors on word-level overlay:\n{combined}",
    );
    assert!(
        !combined.contains("WARNING"),
        "epubcheck emitted warnings on word-level overlay:\n{combined}",
    );
}

fn which(name: &str) -> std::io::Result<std::path::PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{name} not found on PATH"),
    ))
}
