//! Opt-in end-to-end test: convert a real DAISY 2.02 book to EPUB 3 and
//! validate the result with `epubcheck` if it is on `PATH`.
//!
//! Run locally with:
//!
//! ```sh
//! DPUB_TEST_BOOK=/path/to/ncc.html cargo test --test real_conversion -- --nocapture
//! ```
//!
//! Skipped when `DPUB_TEST_BOOK` is unset, so this stays in the regular
//! test suite without needing the book on CI.

use std::path::PathBuf;
use std::process::Command;

use dpub_core::Book;

fn book_path() -> Option<PathBuf> {
    std::env::var_os("DPUB_TEST_BOOK").map(PathBuf::from)
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|d| {
        let p = d.join(name);
        if p.is_file() { Some(p) } else { None }
    })
}

#[test]
fn converts_real_book_to_epub() {
    let Some(ncc) = book_path() else {
        eprintln!("DPUB_TEST_BOOK not set — skipping");
        return;
    };

    let book = Book::from_ncc(&ncc).expect("loading book");
    let dir = tempfile::tempdir().expect("tempdir");
    let epub_path = dir.path().join("converted.epub");

    let publication = dpub_convert::convert(&book).expect("convert");

    // Sanity: every section in the source should round-trip.
    assert_eq!(publication.sections.len(), book.sections.len());
    assert_eq!(publication.audio_files.len(), book.audio_files().len());

    dpub_convert::convert_to_file(&book, &epub_path).expect("write epub");

    let bytes = std::fs::metadata(&epub_path).expect("stat").len();
    assert!(bytes > 0, "epub is empty");
    #[allow(clippy::cast_precision_loss)]
    let mib = bytes as f64 / 1_048_576.0;
    eprintln!("wrote {} ({mib:.1} MiB)", epub_path.display());
}

#[test]
fn epubcheck_clean_on_real_book() {
    let Some(ncc) = book_path() else {
        eprintln!("DPUB_TEST_BOOK not set — skipping");
        return;
    };
    let Some(epubcheck) = which("epubcheck") else {
        eprintln!("epubcheck not on PATH — skipping validation");
        return;
    };

    let book = Book::from_ncc(&ncc).expect("loading book");
    let dir = tempfile::tempdir().expect("tempdir");
    let epub_path = dir.path().join("converted.epub");

    dpub_convert::convert_to_file(&book, &epub_path).expect("write epub");

    let output = Command::new(&epubcheck)
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
    // Warnings are allowed for now (real DAISY books can have edge cases that
    // surface as warnings, e.g. duration drift); errors are the hard line.
}
