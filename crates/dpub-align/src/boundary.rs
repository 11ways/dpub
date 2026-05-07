//! Anchor detection and region classification for the edit script.
//!
//! The two streams won't always cover the same scope. Audiobook
//! preambles ("This is a Luisterpunt production…") are Whisper-only;
//! colophons and indices are ground-truth-only. These mismatches
//! cluster at the boundaries of a section.
//!
//! This module finds the **anchor region** — the longest middle
//! section bracketed by runs of ≥5 consecutive Equal/Fuzzy operations
//! — and tags every op as Leading / Core / Trailing so the timestamp
//! transfer phase can apply different policies per region.

use crate::diff::Op;

/// Minimum run of consecutive Equal/Fuzzy ops required to count as
/// an anchor. Tuned to ignore single coincidental matches inside a
/// preamble (e.g. the book title) while still catching the start of
/// the actual content.
pub(crate) const ANCHOR_MIN_RUN: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    /// Before the leading anchor (or whole script if no anchor).
    Leading,
    /// Inside the anchor region — normal timestamp transfer applies.
    Core,
    /// After the trailing anchor.
    Trailing,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassifiedOp {
    pub op: Op,
    pub region: Region,
}

/// Classify each op by region.
///
/// If no anchor is found (the streams are wildly different), every op
/// is flagged as `Core` so the caller falls back to plain transfer
/// rather than dropping content.
pub(crate) fn classify(script: &[Op]) -> Vec<ClassifiedOp> {
    let leading = first_anchor_start(script);
    let trailing = last_anchor_end(script);

    let (lead_end, trail_start) = match (leading, trailing) {
        (Some(l), Some(t)) if l < t => (l, t),
        // No usable anchor pair: treat everything as Core. This
        // matches the "best effort" contract — when the diff is
        // chaotic, naive transfer is still better than dropping.
        _ => return script.iter().map(|&op| ClassifiedOp {
            op,
            region: Region::Core,
        }).collect(),
    };

    script
        .iter()
        .enumerate()
        .map(|(i, &op)| {
            let region = if i < lead_end {
                Region::Leading
            } else if i >= trail_start {
                Region::Trailing
            } else {
                Region::Core
            };
            ClassifiedOp { op, region }
        })
        .collect()
}

/// Return the index *of the first op* in the leading anchor run
/// (the first match of a ≥ANCHOR_MIN_RUN streak), or `None` if no
/// such run exists.
fn first_anchor_start(script: &[Op]) -> Option<usize> {
    let mut run_len = 0usize;
    let mut run_start = 0usize;
    for (i, op) in script.iter().enumerate() {
        if is_match(op) {
            if run_len == 0 {
                run_start = i;
            }
            run_len += 1;
            if run_len >= ANCHOR_MIN_RUN {
                return Some(run_start);
            }
        } else {
            run_len = 0;
        }
    }
    None
}

/// Return the index *one past the last op* in the trailing anchor
/// run, or `None` if no such run exists.
fn last_anchor_end(script: &[Op]) -> Option<usize> {
    let mut run_len = 0usize;
    let mut run_end = 0usize; // exclusive
    for (i, op) in script.iter().enumerate().rev() {
        if is_match(op) {
            if run_len == 0 {
                run_end = i + 1;
            }
            run_len += 1;
            if run_len >= ANCHOR_MIN_RUN {
                return Some(run_end);
            }
        } else {
            run_len = 0;
        }
    }
    None
}

fn is_match(op: &Op) -> bool {
    matches!(op, Op::Equal { .. } | Op::Fuzzy { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(w: usize, g: usize) -> Op {
        Op::Equal {
            whisper_idx: w,
            gt_idx: g,
        }
    }
    fn del(w: usize) -> Op {
        Op::Delete { whisper_idx: w }
    }
    fn ins(g: usize) -> Op {
        Op::Insert { gt_idx: g }
    }

    #[test]
    fn no_anchors_means_all_core() {
        // Only 4 matches — below threshold of 5
        let script = vec![eq(0, 0), eq(1, 1), eq(2, 2), eq(3, 3), del(4)];
        let classified = classify(&script);
        assert!(classified.iter().all(|c| c.region == Region::Core));
    }

    #[test]
    fn preamble_is_leading() {
        // 10 deletes (preamble), then 5 matches (anchor).
        let mut script: Vec<Op> = (0..10).map(del).collect();
        script.extend((0..5).map(|i| eq(10 + i, i)));
        let classified = classify(&script);
        assert_eq!(classified.len(), 15);
        // The 10 deletes should be Leading; the 5 matches Core.
        for c in &classified[0..10] {
            assert_eq!(c.region, Region::Leading);
        }
        for c in &classified[10..15] {
            assert_eq!(c.region, Region::Core);
        }
    }

    #[test]
    fn colophon_is_trailing() {
        // 5 matches (anchor), then 10 inserts (colophon).
        let mut script: Vec<Op> = (0..5).map(|i| eq(i, i)).collect();
        script.extend((5..15).map(ins));
        let classified = classify(&script);
        for c in &classified[0..5] {
            assert_eq!(c.region, Region::Core);
        }
        for c in &classified[5..15] {
            assert_eq!(c.region, Region::Trailing);
        }
    }

    #[test]
    fn full_book_pattern() {
        // preamble (8 deletes) → core (5 matches, 1 delete, 5 matches)
        // → outro (8 deletes)
        let mut script: Vec<Op> = (0..8).map(del).collect();
        script.extend((0..5).map(|i| eq(8 + i, i)));
        script.push(del(13));
        script.extend((5..10).map(|i| eq(14 + i - 5, i)));
        script.extend((19..27).map(del));

        let classified = classify(&script);
        // First 8 should be Leading
        assert!(classified[0..8].iter().all(|c| c.region == Region::Leading));
        // Last 8 should be Trailing
        let n = classified.len();
        assert!(classified[n - 8..n].iter().all(|c| c.region == Region::Trailing));
        // Middle should be Core
        assert!(classified[8..n - 8].iter().all(|c| c.region == Region::Core));
    }

    #[test]
    fn single_coincidental_match_in_preamble_doesnt_anchor() {
        // 3 deletes, 1 match (book title in preamble?), 4 deletes,
        // then 5 real matches.
        let mut script: Vec<Op> = vec![del(0), del(1), del(2)];
        script.push(eq(3, 0)); // coincidental
        script.extend((4..8).map(del));
        script.extend((0..5).map(|i| eq(8 + i, i + 1)));
        let classified = classify(&script);
        // The coincidental match should be classified as Leading,
        // because it's before the real 5-run anchor.
        assert_eq!(classified[3].region, Region::Leading);
        // Real anchor begins at index 8.
        for c in &classified[8..13] {
            assert_eq!(c.region, Region::Core);
        }
    }
}
