//! Dry-run helper: parse a DAISY 2.02 publication and a ground-truth
//! file and report how many sections the heading matcher resolves —
//! without running Whisper. Useful when validating a new ground-truth
//! file against a book.
//!
//! Usage:
//! ```text
//! cargo run --release -p dpub-align --example match_sections -- \
//!   /path/to/book/ncc.html /path/to/groundtruth.{txt,md,json}
//! ```

use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let ncc = args.next().expect("usage: match_sections <ncc.html> <ground-truth>");
    let gt = args.next().expect("usage: match_sections <ncc.html> <ground-truth>");

    let book = dpub_core::Book::from_ncc(Path::new(&ncc)).expect("parse DAISY");
    let raw = std::fs::read_to_string(&gt).expect("read ground truth");

    let headings: Vec<(&str, usize)> = book
        .master
        .references
        .iter()
        .enumerate()
        .map(|(i, r)| (r.title.as_str(), i))
        .collect();

    let sections = dpub_align::split_into_sections(&raw, &headings);
    println!(
        "Matched {} of {} DAISY sections",
        sections.len(),
        headings.len()
    );
    println!();

    let matched: std::collections::HashSet<usize> = sections.iter().map(|s| s.ncc_index).collect();
    println!("First 10 matches:");
    for s in sections.iter().take(10) {
        let title = headings[s.ncc_index].0;
        let preview: String = s.text.chars().take(50).collect::<String>().replace('\n', " ");
        println!(
            "  [{:3}] {:30}  → {:5} chars  {:?}",
            s.ncc_index,
            title,
            s.text.len(),
            preview
        );
    }
    println!();

    let unmatched: Vec<&str> = headings
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched.contains(i))
        .map(|(_, (t, _))| *t)
        .collect();
    println!("Unmatched headings ({} total):", unmatched.len());
    for t in unmatched.iter().take(20) {
        println!("  {t}");
    }
    if unmatched.len() > 20 {
        println!("  ... and {} more", unmatched.len() - 20);
    }
}
