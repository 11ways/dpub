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
    /// Whisper word and ground truth word at the same position but
    /// not similar enough to count as a fuzzy match (e.g. number
    /// "2024" vs spoken form, name swaps, low-similarity errors).
    /// Still a 1:1 positional pairing — the narrator said *something*
    /// here, the book has *something* here, and the time span Whisper
    /// produced for that audio chunk is the best estimate we have for
    /// the GT word's timing.
    Replace {
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
                        // Position-paired but not similar enough to
                        // call a fuzzy match. Emit as a single
                        // Replace so transfer.rs can use Whisper's
                        // time span for the GT word — the narrator
                        // and the book are at the same position even
                        // if the spelling diverges.
                        script.push(Op::Replace {
                            whisper_idx: w_i,
                            gt_idx: g_i,
                            score,
                        });
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
    coalesce_adjacent_delete_insert(&mut script, &whisper_key_refs, &gt_keys);
    script
}

/// Post-pass: pair adjacent Delete-runs with Insert-runs into Replace
/// (or Fuzzy) ops. Myers diff sometimes emits these as separate blocks
/// when the optimal edit-distance path doesn't classify them as a
/// `Replace` together. From our perspective they're positionally
/// paired — the audio span Whisper covered with `Delete` words is the
/// same span the narrator was reading the corresponding `Insert`
/// words. Without this pass, every such GT word ends up as a pure
/// `Insert`, gets interpolation-collapsed to zero duration, and is
/// filtered out of the SMIL by the MED-009 guard.
fn coalesce_adjacent_delete_insert(
    script: &mut Vec<Op>,
    whisper_keys: &[&str],
    gt_keys: &[&str],
) {
    let mut i = 0;
    let mut out: Vec<Op> = Vec::with_capacity(script.len());
    while i < script.len() {
        // Collect a run of consecutive Deletes (and any Inserts that
        // immediately follow). The order Myers emits is typically
        // "Delete-run, Insert-run" within a divergence, but we accept
        // either order — the positional pairing is what matters.
        let run_start = i;
        let mut deletes: Vec<usize> = Vec::new();
        let mut inserts: Vec<usize> = Vec::new();
        while i < script.len() {
            match script[i] {
                Op::Delete { whisper_idx } => {
                    deletes.push(whisper_idx);
                    i += 1;
                }
                Op::Insert { gt_idx } => {
                    inserts.push(gt_idx);
                    i += 1;
                }
                _ => break,
            }
        }
        if deletes.is_empty() || inserts.is_empty() {
            // Run was pure Delete or pure Insert — nothing to pair.
            out.extend_from_slice(&script[run_start..i]);
            continue;
        }
        // Pair up to min length. Pair-position 0 maps deletes[0] to
        // inserts[0]; the leftover side stays as standalone Delete /
        // Insert ops at the end of the run.
        let pair_len = deletes.len().min(inserts.len());
        for j in 0..pair_len {
            let w_i = deletes[j];
            let g_i = inserts[j];
            let score = strsim::jaro_winkler(whisper_keys[w_i], gt_keys[g_i]);
            if score >= FUZZY_THRESHOLD {
                out.push(Op::Fuzzy {
                    whisper_idx: w_i,
                    gt_idx: g_i,
                    score,
                });
            } else {
                out.push(Op::Replace {
                    whisper_idx: w_i,
                    gt_idx: g_i,
                    score,
                });
            }
        }
        for j in pair_len..deletes.len() {
            out.push(Op::Delete {
                whisper_idx: deletes[j],
            });
        }
        for j in pair_len..inserts.len() {
            out.push(Op::Insert { gt_idx: inserts[j] });
        }
    }
    *script = out;
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
    fn unrelated_words_at_same_position_emit_replace() {
        // "table" vs "elephant" — Jaro-Winkler below the fuzzy
        // threshold but the words are positionally paired, so the
        // diff emits a single Replace op (not Delete + Insert).
        // This lets transfer.rs reuse Whisper's time span for the
        // ground truth word.
        let w = vec![ww("table", 0.0, 1.0)];
        let g = gt(&["elephant"]);
        let ops = diff_words(&w, &g);
        assert_eq!(ops.len(), 1);
        match ops[0] {
            Op::Replace { score, .. } => assert!(score < 0.85),
            other => panic!("expected Replace, got {other:?}"),
        }
    }
}
