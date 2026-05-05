//! Network-gated smoke test for the Open Library cover lookup.
//!
//! Skipped unless `DPUB_TEST_OPENLIBRARY=1` is set. Real CI runs (and
//! every contributor's `cargo test`) skip silently — we don't want
//! the test suite to fail because Open Library was briefly slow,
//! degraded, or because someone is offline.
//!
//! Run with:
//!
//! ```sh
//! DPUB_TEST_OPENLIBRARY=1 cargo test -p dpub-meta --test openlibrary -- --nocapture
//! ```
//!
//! Asserts only that the path *can* return a real JPEG/PNG cover for
//! a known well-indexed book — not the exact bytes, since Open Library
//! image content can change.

use dpub_meta::{LookupHints, lookup_cover};

fn enabled() -> bool {
    std::env::var_os("DPUB_TEST_OPENLIBRARY").is_some()
}

#[test]
fn fetches_cover_for_pride_and_prejudice_by_isbn() {
    if !enabled() {
        eprintln!("DPUB_TEST_OPENLIBRARY not set — skipping");
        return;
    }
    let hints = LookupHints {
        title: "Pride and Prejudice",
        creator: Some("Jane Austen"),
        language: Some("eng"),
        identifier: Some("9780141439518"),
    };
    let got = lookup_cover(&hints).expect("lookup");
    let cover = got.expect("matched");
    assert!(cover.bytes.len() > 1024, "cover too small: {}", cover.bytes.len());
    assert!(
        cover.media_type == "image/jpeg" || cover.media_type == "image/png",
        "unexpected media type: {}",
        cover.media_type,
    );
    eprintln!(
        "got {} bytes ({}); provenance: {}",
        cover.bytes.len(),
        cover.media_type,
        cover.provenance,
    );
}

#[test]
fn fetches_cover_by_title_and_author_when_no_isbn() {
    if !enabled() {
        eprintln!("DPUB_TEST_OPENLIBRARY not set — skipping");
        return;
    }
    let hints = LookupHints {
        title: "Pride and Prejudice",
        creator: Some("Jane Austen"),
        language: Some("eng"),
        identifier: None, // force the title+author path
    };
    let got = lookup_cover(&hints).expect("lookup");
    let cover = got.expect("matched");
    assert!(cover.bytes.len() > 1024);
    eprintln!(
        "got {} bytes via {} path",
        cover.bytes.len(),
        cover.provenance,
    );
}

#[test]
fn returns_none_for_obviously_nonexistent_book() {
    if !enabled() {
        eprintln!("DPUB_TEST_OPENLIBRARY not set — skipping");
        return;
    }
    let hints = LookupHints {
        title: "ZZZZZZZZZZZZZZ_DPUB_TEST_NONEXISTENT_BOOK_XXXXX",
        creator: Some("AAAAAAAAA Nonexistent Author"),
        language: None,
        identifier: None,
    };
    let got = lookup_cover(&hints).expect("lookup");
    assert!(
        got.is_none(),
        "expected miss for gibberish title, got {:?}",
        got.as_ref().map(|c| &c.provenance),
    );
}
