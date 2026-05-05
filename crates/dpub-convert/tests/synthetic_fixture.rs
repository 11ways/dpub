//! End-to-end conversion of an in-tree synthetic DAISY 2.02 book.
//!
//! Unlike `real_conversion.rs` (which is gated on `DPUB_TEST_BOOK` because
//! the reference Vlaams audiobook is third-party copyright), this test
//! ships a tiny redistributable fixture inside the repo and runs on every
//! `cargo test` invocation, including CI. It verifies the full pipeline
//! (parse → build Publication → write ZIP) end-to-end against fresh code.
//!
//! Optional EPUBCheck assertion fires when `epubcheck` is on PATH; skipped
//! otherwise so CI runners without it stay green.

use std::path::{Path, PathBuf};
use std::process::Command;

use dpub_core::Book;

const FIXTURE: &str = "tests/fixtures/minimal_daisy";

fn fixture_ncc() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE)
        .join("ncc.html")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|d| {
        let p = d.join(name);
        if p.is_file() { Some(p) } else { None }
    })
}

#[test]
fn parses_minimal_daisy_book() {
    let book = Book::from_ncc(fixture_ncc()).expect("parse fixture");
    assert_eq!(book.master.references.len(), 1);
    assert_eq!(book.sections.len(), 1);
    assert_eq!(book.audio_files().len(), 1);
    assert_eq!(book.total_par_count(), 2);
    assert_eq!(book.total_audio_clip_count(), 2);
    let metadata = book.metadata();
    assert_eq!(metadata.title.as_deref(), Some("Synthetic DAISY Test Book"));
    assert_eq!(metadata.language.as_deref(), Some("en"));
}

#[test]
fn converts_minimal_daisy_book_to_epub() {
    let book = Book::from_ncc(fixture_ncc()).expect("parse fixture");
    let dir = tempfile::tempdir().expect("tempdir");
    let epub = dir.path().join("minimal.epub");

    dpub_convert::convert_to_file(&book, &epub, &dpub_convert::ConvertOptions::default())
        .expect("convert");

    let bytes = std::fs::metadata(&epub).expect("stat").len();
    assert!(bytes > 0, "epub is empty");

    // Spot-check the archive contents instead of just trusting the writer.
    let f = std::fs::File::open(&epub).expect("open");
    let mut zip = zip::ZipArchive::new(f).expect("zip");
    let names: std::collections::BTreeSet<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_owned())
        .collect();
    for required in [
        "mimetype",
        "META-INF/container.xml",
        "EPUB/package.opf",
        "EPUB/nav.xhtml",
        "EPUB/content/ptk000001.xhtml",
        "EPUB/media-overlays/ptk000001.smil",
        "EPUB/audio/audio.mp3",
    ] {
        assert!(
            names.contains(required),
            "missing entry: {required} (have {names:?})"
        );
    }
}

#[test]
fn epubcheck_clean_on_minimal_book() {
    let Some(epubcheck) = which("epubcheck") else {
        eprintln!("epubcheck not on PATH — skipping validation");
        return;
    };

    let book = Book::from_ncc(fixture_ncc()).expect("parse fixture");
    let dir = tempfile::tempdir().expect("tempdir");
    let epub = dir.path().join("minimal.epub");
    dpub_convert::convert_to_file(&book, &epub, &dpub_convert::ConvertOptions::default())
        .expect("convert");

    let output = Command::new(&epubcheck)
        .arg(&epub)
        .output()
        .expect("run epubcheck");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        output.status.success(),
        "epubcheck reported errors:\n{combined}"
    );
    assert!(
        !combined.contains("WARNING"),
        "epubcheck emitted warnings:\n{combined}"
    );
}
