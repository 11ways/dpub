//! Opt-in integration test against a real DAISY 2.02 book on disk.
//!
//! Run locally with:
//!
//! ```sh
//! DPUB_TEST_BOOK=/path/to/ncc.html cargo test --test real_book -- --nocapture
//! ```
//!
//! When `DPUB_TEST_BOOK` is unset (e.g. on CI) the test is silently skipped,
//! so we can keep this in the standard test suite without distributing the
//! book itself.

use std::path::PathBuf;

use dpub_core::{Book, MasterSmil, SectionSmil, write_master_smil, write_section_smil};

fn book_path() -> Option<PathBuf> {
    std::env::var_os("DPUB_TEST_BOOK").map(PathBuf::from)
}

#[test]
fn parses_ontmoetingen_book() {
    let Some(ncc) = book_path() else {
        eprintln!("DPUB_TEST_BOOK not set — skipping integration test");
        return;
    };

    let book = Book::from_ncc(&ncc).expect("loading book");

    // High-level invariants taken from the NCC's own metadata claims.
    let m = book.metadata();
    assert!(m.title.is_some(), "title missing");
    assert!(m.total_time.is_some(), "ncc:totalTime missing");

    // Number of sections in master.smil must match the count of headings in NCC.
    assert_eq!(
        book.master.references.len(),
        book.sections.len(),
        "section count != parsed-section count",
    );

    // ncc:tocItems should be heading + page count, which equals our par count.
    if let Some(toc_items) = m.toc_items {
        assert_eq!(
            book.total_par_count(),
            toc_items as usize,
            "par count mismatches ncc:tocItems",
        );
    }

    // Audio total should match ncc:totalTime to within a second.
    let claimed = m.total_time.as_deref().and_then(parse_hms_seconds);
    if let Some(claimed_secs) = claimed {
        let measured = book.total_audio_seconds();
        let diff = (measured - claimed_secs).abs();
        assert!(
            diff < 2.0,
            "audio total {measured}s differs from claimed {claimed_secs}s by {diff}s",
        );
    }
}

#[test]
fn round_trip_smil_files_preserve_ast() {
    let Some(ncc) = book_path() else {
        eprintln!("DPUB_TEST_BOOK not set — skipping integration test");
        return;
    };

    let book = Book::from_ncc(&ncc).expect("loading book");

    // master.smil round-trip
    let serialised = write_master_smil(&book.master);
    let parsed_again = MasterSmil::parse_bytes(
        serialised.as_bytes(),
        std::path::Path::new("(round-trip).smil"),
    )
    .expect("re-parse master");
    assert_eq!(
        book.master, parsed_again,
        "master.smil round trip changed AST",
    );

    // every per-section SMIL round-trip
    for (idx, section) in book.sections.iter().enumerate() {
        let serialised = write_section_smil(section);
        let parsed_again = SectionSmil::parse_bytes(
            serialised.as_bytes(),
            std::path::Path::new("(round-trip).smil"),
        )
        .unwrap_or_else(|e| panic!("re-parse section {idx} failed: {e}"));
        assert_eq!(
            section, &parsed_again,
            "section {idx} round trip changed AST",
        );
    }
}

fn parse_hms_seconds(s: &str) -> Option<f64> {
    let mut parts = s.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let secs: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + secs)
}
