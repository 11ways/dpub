//! Word-level Myers diff with Jaro-Winkler fuzzy promotion.
//!
//! Produces an [`EditScript`] — a sequence of [`Op`] entries that
//! describe how to turn the Whisper word stream into the ground truth
//! word stream (Equal/Fuzzy/Insert/Delete/Replace).

use similar::{capture_diff_slices, Algorithm};

use crate::{GroundTruthWord, WordTiming};

/// One operation in the edit script. Indices refer to the input
/// slices: `whisper_idx` into the Whisper word stream, `gt_idx` into
/// the ground truth word stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    /// Whisper word and ground truth word match exactly (after
    /// normalisation).
    Equal { whisper_idx: usize, gt_idx: usize },
    /// Whisper word and ground truth word are similar enough
    /// (Jaro-Winkler ≥ threshold) — treat as a match.
    Fuzzy {
        whisper_idx: usize,
        gt_idx: usize,
        score: f64,
    },
    /// Whisper word with no ground truth counterpart (hallucinated /
    /// repeated / preamble).
    Delete { whisper_idx: usize },
    /// Ground truth word with no Whisper counterpart (skipped /
    /// colophon / outro).
    Insert { gt_idx: usize },
}

/// Threshold above which a Replace operation is promoted to Fuzzy.
/// Set lower than the typical 0.9 to catch trailing-letter truncations
/// like "Antwerpe" → "Antwerpen" (≈ 0.97) and minor letter swaps.
const FUZZY_THRESHOLD: f64 = 0.85;

/// Run Myers diff over the normalised keys of both streams and emit
/// the post-processed edit script (Equal/Fuzzy/Insert/Delete).
pub(crate) fn diff_words(
    whisper: &[WordTiming],
    ground_truth: &[GroundTruthWord],
) -> Vec<Op> {
    use crate::normalize;

    // Build key slices. We could compute keys lazily but caching
    // avoids re-running normalise() inside Myers' inner loop.
    let whisper_keys: Vec<String> = whisper.iter().map(|w| normalize::normalise(&w.text)).collect();
    let gt_keys: Vec<&str> = ground_truth.iter().map(|w| w.key.as_str()).collect();
    let whisper_key_refs: Vec<&str> = whisper_keys.iter().map(String::as_str).collect();

    let diff_ops = capture_diff_slices(Algorithm::Myers, &whisper_key_refs, &gt_keys);

    let mut script: Vec<Op> = Vec::with_capacity(whisper.len() + ground_truth.len());
    for op in diff_ops {
        match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    script.push(Op::Equal {
                        whisper_idx: old_index + i,
                        gt_idx: new_index + i,
                    });
                }
            }
            similar::DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..old_len {
                    script.push(Op::Delete {
                        whisper_idx: old_index + i,
                    });
                }
            }
            similar::DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..new_len {
                    script.push(Op::Insert {
                        gt_idx: new_index + i,
                    });
                }
            }
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                // Pair up the Replace block one-to-one (longest common
                // length); leftover Whisper words become Deletes,
                // leftover ground truth words become Inserts. Within
                // each pair, promote to Fuzzy if Jaro-Winkler is high
                // enough — handles "Antwerpe"/"Antwerpen", trailing-s
                // confusions, etc.
                let pair_len = old_len.min(new_len);
                for i in 0..pair_len {
                    let w_i = old_index + i;
                    let g_i = new_index + i;
                    let score =
                        strsim::jaro_winkler(whisper_key_refs[w_i], gt_keys[g_i]);
                    if score >= FUZZY_THRESHOLD {
                        script.push(Op::Fuzzy {
                            whisper_idx: w_i,
                            gt_idx: g_i,
                            score,
                        });
                    } else {
                        // Genuinely different word: emit as
                        // Delete + Insert. The transfer phase uses
                        // Whisper's time span for the inserted word
                        // anyway when these are adjacent.
                        script.push(Op::Delete { whisper_idx: w_i });
                        script.push(Op::Insert { gt_idx: g_i });
                    }
                }
                for i in pair_len..old_len {
                    script.push(Op::Delete {
                        whisper_idx: old_index + i,
                    });
                }
                for i in pair_len..new_len {
                    script.push(Op::Insert {
                        gt_idx: new_index + i,
                    });
                }
            }
        }
    }
    script
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize;

    fn ww(text: &str, start: f64, end: f64) -> WordTiming {
        WordTiming {
            start_seconds: start,
            end_seconds: end,
            text: text.to_owned(),
        }
    }

    fn gt(words: &[&str]) -> Vec<GroundTruthWord> {
        words
            .iter()
            .map(|s| GroundTruthWord {
                text: (*s).to_owned(),
                key: normalize::normalise(s),
            })
            .collect()
    }

    #[test]
    fn perfect_match_all_equal() {
        let w = vec![
            ww("hello", 0.0, 0.5),
            ww("world", 0.5, 1.0),
        ];
        let g = gt(&["hello", "world"]);
        let ops = diff_words(&w, &g);
        assert!(matches!(ops[0], Op::Equal { .. }));
        assert!(matches!(ops[1], Op::Equal { .. }));
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn whisper_hallucination_is_delete() {
        let w = vec![
            ww("hello", 0.0, 0.5),
            ww("um", 0.5, 0.7),
            ww("world", 0.7, 1.2),
        ];
        let g = gt(&["hello", "world"]);
        let ops = diff_words(&w, &g);
        // Should have an Equal, a Delete, an Equal.
        let deletes: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, Op::Delete { .. }))
            .collect();
        assert_eq!(deletes.len(), 1);
    }

    #[test]
    fn whisper_omission_is_insert() {
        let w = vec![
            ww("hello", 0.0, 0.5),
            ww("world", 0.5, 1.0),
        ];
        let g = gt(&["hello", "the", "world"]);
        let ops = diff_words(&w, &g);
        let inserts: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, Op::Insert { .. }))
            .collect();
        assert_eq!(inserts.len(), 1);
    }

    #[test]
    fn truncated_word_promoted_to_fuzzy() {
        // "Antwerpe" vs "Antwerpen" — Jaro-Winkler ~0.97
        let w = vec![ww("antwerpe", 0.0, 1.0)];
        let g = gt(&["antwerpen"]);
        let ops = diff_words(&w, &g);
        assert_eq!(ops.len(), 1);
        match ops[0] {
            Op::Fuzzy { score, .. } => assert!(score >= 0.85),
            other => panic!("expected Fuzzy, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_words_stay_replace_split() {
        // "table" vs "elephant" — Jaro-Winkler well below threshold
        let w = vec![ww("table", 0.0, 1.0)];
        let g = gt(&["elephant"]);
        let ops = diff_words(&w, &g);
        // Should be Delete + Insert, not Fuzzy
        assert!(ops.iter().any(|op| matches!(op, Op::Delete { .. })));
        assert!(ops.iter().any(|op| matches!(op, Op::Insert { .. })));
        assert!(!ops.iter().any(|op| matches!(op, Op::Fuzzy { .. })));
    }
}
