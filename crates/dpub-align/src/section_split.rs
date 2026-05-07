//! Split a single ground truth file into per-section text by matching
//! its headings against the DAISY NCC headings.
//!
//! Auto-detects markdown vs plain text:
//! - if any line starts with `#` followed by space, treat as markdown
//!   and use those as candidate headings
//! - otherwise scan every short line and fuzzy-match against the
//!   provided NCC heading texts
//!
//! Match score: Jaro-Winkler over normalised heading text.

use crate::normalize;

/// One section's slice of the ground truth text.
#[derive(Debug, Clone)]
pub struct SectionText {
    /// Index into the NCC headings list (0-based).
    pub ncc_index: usize,
    /// Raw text between this heading and the next matched heading.
    /// Paragraph boundaries (blank lines) are preserved.
    pub text: String,
}

/// Threshold for matching a candidate heading line to an NCC heading.
const HEADING_MATCH_THRESHOLD: f64 = 0.85;

/// Maximum character length for a "candidate heading" line in plain
/// text mode. Real chapter titles are short; long lines are body text.
const PLAIN_HEADING_MAX_LEN: usize = 120;

/// Split the ground truth `text` into per-section chunks by matching
/// headings against the supplied NCC heading texts (in order).
///
/// `ncc_headings` is a slice of (heading_text, ncc_index) tuples. The
/// `ncc_index` lets the caller map results back to its own data
/// structures.
pub fn split_into_sections(
    text: &str,
    ncc_headings: &[(&str, usize)],
) -> Vec<SectionText> {
    let is_markdown = text
        .lines()
        .any(|l| l.trim_start().starts_with('#') && l.trim_start().chars().nth(1) == Some(' '));

    let candidate_lines = if is_markdown {
        markdown_headings(text)
    } else {
        plain_text_headings(text)
    };

    // For each NCC heading (in order), find the *first* candidate line
    // (after the previous match) that fuzzy-matches it. Locking matches
    // in document order prevents a later-section heading from stealing
    // an earlier section's match.
    let mut matches: Vec<(usize, usize, usize)> = Vec::new(); // (ncc_idx, line_byte_offset_start, line_byte_offset_end)
    let mut search_from: usize = 0;
    for (heading_text, ncc_idx) in ncc_headings {
        let target = normalize_heading(heading_text);
        let mut best: Option<(f64, usize, usize)> = None;
        for cand in &candidate_lines {
            if cand.line_start < search_from {
                continue;
            }
            let normalised = normalize_heading(&cand.heading_text);
            if normalised.is_empty() {
                continue;
            }
            let score = strsim::jaro_winkler(&target, &normalised);
            if score >= HEADING_MATCH_THRESHOLD
                && best.map_or(true, |(prev, _, _)| score > prev)
            {
                best = Some((score, cand.line_start, cand.line_end));
            }
            // Don't break on first hit — we want the best score
            // before search_from advances.
            // But cap search distance so a typo doesn't cause a match
            // 30 chapters later: stop once we've scanned enough lines.
            if cand.line_start > search_from + 200_000 {
                break;
            }
        }
        if let Some((_, start, end)) = best {
            matches.push((*ncc_idx, start, end));
            search_from = end;
        }
    }

    // Now slice the text between matched heading line ends.
    let bytes = text.as_bytes();
    let mut sections = Vec::with_capacity(matches.len());
    for i in 0..matches.len() {
        let (ncc_idx, _heading_start, heading_end) = matches[i];
        let body_start = heading_end;
        let body_end = matches.get(i + 1).map_or(bytes.len(), |&(_, next_start, _)| next_start);
        if body_end <= body_start {
            continue;
        }
        let slice = &text[body_start..body_end];
        sections.push(SectionText {
            ncc_index: ncc_idx,
            text: slice.trim().to_owned(),
        });
    }
    sections
}

#[derive(Debug)]
struct CandidateHeading {
    heading_text: String,
    line_start: usize,
    line_end: usize,
}

fn markdown_headings(text: &str) -> Vec<CandidateHeading> {
    let mut out = Vec::new();
    let mut offset: usize = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            if let Some(rest) = trimmed.strip_prefix('#') {
                let heading_body = rest.trim_start_matches('#').trim();
                if !heading_body.is_empty() {
                    out.push(CandidateHeading {
                        heading_text: heading_body.to_owned(),
                        line_start: offset,
                        line_end: offset + line.len(),
                    });
                }
            }
        }
        offset += line.len();
    }
    out
}

fn plain_text_headings(text: &str) -> Vec<CandidateHeading> {
    // Heuristic: any non-empty line ≤ PLAIN_HEADING_MAX_LEN is a
    // candidate. We rely on the fuzzy-match threshold to filter out
    // body-text lines that happen to be short.
    let mut out = Vec::new();
    let mut offset: usize = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && trimmed.len() <= PLAIN_HEADING_MAX_LEN {
            out.push(CandidateHeading {
                heading_text: trimmed.to_owned(),
                line_start: offset,
                line_end: offset + line.len(),
            });
        }
        offset += line.len();
    }
    out
}

/// Heading-specific normalisation: lowercase, strip leading numbering
/// ("Chapter 1", "1.", "I."), collapse whitespace.
fn normalize_heading(text: &str) -> String {
    let trimmed = text.trim();
    let stripped = strip_leading_numbering(trimmed);
    // Strip surrounding punctuation per word, then rejoin.
    let cleaned: Vec<String> = stripped
        .split_whitespace()
        .map(normalize::normalise)
        .filter(|s| !s.is_empty())
        .collect();
    cleaned.join(" ")
}

fn strip_leading_numbering(s: &str) -> &str {
    // Strip patterns like "1. ", "1) ", "Chapter 1: ", "I. ".
    let s = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')');
    s.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_markdown_headings() {
        let text = "\
# Chapter 1
First paragraph.

## Section A
Second paragraph.

# Chapter 2
Third paragraph.
";
        let ncc = [("Chapter 1", 0), ("Chapter 2", 1)];
        let sections = split_into_sections(text, &ncc);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].ncc_index, 0);
        assert!(sections[0].text.contains("First paragraph"));
        assert!(sections[0].text.contains("Section A"));
        assert!(sections[0].text.contains("Second paragraph"));
        assert_eq!(sections[1].ncc_index, 1);
        assert!(sections[1].text.contains("Third paragraph"));
    }

    #[test]
    fn detects_plain_text_headings() {
        let text = "\
Chapter 1

This is the first paragraph of chapter one.

Chapter 2

This is the first paragraph of chapter two.
";
        let ncc = [("Chapter 1", 0), ("Chapter 2", 1)];
        let sections = split_into_sections(text, &ncc);
        assert_eq!(sections.len(), 2);
        assert!(sections[0].text.contains("first paragraph of chapter one"));
        assert!(sections[1].text.contains("first paragraph of chapter two"));
    }

    #[test]
    fn fuzzy_matches_typo() {
        // "Hofdstuk 1" (typo for "Hoofdstuk 1") should still match
        // via Jaro-Winkler.
        let text = "\
# Hofdstuk 1
Body text.

# Hoofdstuk 2
More body text.
";
        let ncc = [("Hoofdstuk 1", 0), ("Hoofdstuk 2", 1)];
        let sections = split_into_sections(text, &ncc);
        assert_eq!(sections.len(), 2);
    }

    #[test]
    fn unmatched_heading_skipped() {
        let text = "\
# Chapter 1
Body text.
";
        let ncc = [("Chapter 1", 0), ("Chapter 2 — Not in text", 1)];
        let sections = split_into_sections(text, &ncc);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].ncc_index, 0);
    }

    #[test]
    fn handles_empty_input() {
        let text = "";
        let ncc = [("Chapter 1", 0)];
        let sections = split_into_sections(text, &ncc);
        assert!(sections.is_empty());
    }
}
