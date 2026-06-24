//! Walk the classified edit script and produce one [`AlignedWord`]
//! per ground truth word, with timestamps transferred from Whisper
//! (or interpolated / bracketed / unsynced as appropriate).
//!
//! The walk is index-based on the script. Two cursors track which
//! input slice each op refers to:
//! - `whisper_words[op.whisper_idx]` for the Whisper time data
//! - `gt_words[op.gt_idx]` for the ground truth surface text

use crate::boundary::{ClassifiedOp, Region};
use crate::diff::Op;
use crate::{
    AlignedWord, BoundaryStrategy, Confidence, GroundTruthWord, TrimEvent, TrimKind, WordTiming,
};

/// Produce the aligned word stream and trim diagnostics.
pub(crate) fn transfer_timestamps(
    whisper: &[WordTiming],
    ground_truth: &[GroundTruthWord],
    script: &[ClassifiedOp],
    strategy: BoundaryStrategy,
) -> (Vec<AlignedWord>, Vec<TrimEvent>) {
    // Pass 1: assemble per-region operation buckets.
    let mut leading: Vec<&ClassifiedOp> = Vec::new();
    let mut core: Vec<&ClassifiedOp> = Vec::new();
    let mut trailing: Vec<&ClassifiedOp> = Vec::new();
    for c in script {
        match c.region {
            Region::Leading => leading.push(c),
            Region::Core => core.push(c),
            Region::Trailing => trailing.push(c),
        }
    }

    let mut aligned: Vec<AlignedWord> = Vec::with_capacity(ground_truth.len());
    let mut trim_log: Vec<TrimEvent> = Vec::new();

    // Time-bracket helpers: the leading region spans audio time from 0
    // to the first core anchor's start; the trailing from last core
    // anchor's end to whisper.last().end.
    let core_start_seconds = first_core_match_time(&core, whisper).unwrap_or(0.0);
    let core_end_seconds = last_core_match_time(&core, whisper)
        .unwrap_or_else(|| whisper.last().map_or(0.0, |w| w.end_seconds));
    let total_audio_end = whisper.last().map_or(core_end_seconds, |w| w.end_seconds);

    handle_boundary_region(
        &leading,
        whisper,
        ground_truth,
        strategy,
        BoundaryEnd::Leading,
        0.0,
        core_start_seconds,
        &mut aligned,
        &mut trim_log,
    );

    transfer_core(&core, whisper, ground_truth, &mut aligned);

    handle_boundary_region(
        &trailing,
        whisper,
        ground_truth,
        strategy,
        BoundaryEnd::Trailing,
        core_end_seconds,
        total_audio_end,
        &mut aligned,
        &mut trim_log,
    );

    enforce_monotonicity(&mut aligned);
    (aligned, trim_log)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundaryEnd {
    Leading,
    Trailing,
}

#[allow(clippy::too_many_arguments)]
fn handle_boundary_region(
    region: &[&ClassifiedOp],
    whisper: &[WordTiming],
    ground_truth: &[GroundTruthWord],
    strategy: BoundaryStrategy,
    end: BoundaryEnd,
    gap_start: f64,
    gap_end: f64,
    aligned: &mut Vec<AlignedWord>,
    trim_log: &mut Vec<TrimEvent>,
) {
    if region.is_empty() {
        return;
    }

    // Whisper-only words in a boundary region are always discarded
    // (audiobook preamble / outro). Their time is *not* redistributed.
    // Replace ops in a boundary region also discard their Whisper
    // word — outside the anchor region we can't trust positional
    // pairing, so the safer choice is to drop both halves instead of
    // binding a real audio time to a GT word that might never have
    // been read.
    let whisper_only: Vec<usize> = region
        .iter()
        .filter_map(|c| match c.op {
            Op::Delete { whisper_idx } | Op::Replace { whisper_idx, .. } => Some(whisper_idx),
            _ => None,
        })
        .collect();
    if !whisper_only.is_empty() {
        let preview = preview_from_whisper(whisper, &whisper_only);
        trim_log.push(TrimEvent {
            kind: match end {
                BoundaryEnd::Leading => TrimKind::LeadingWhisper,
                BoundaryEnd::Trailing => TrimKind::TrailingWhisper,
            },
            word_count: whisper_only.len(),
            preview,
        });
    }

    // Ground-truth-only words: collect, then apply strategy. Replace
    // contributes its GT side here for the same reason: outside the
    // anchor region, the positional pairing isn't load-bearing.
    let gt_only: Vec<usize> = region
        .iter()
        .filter_map(|c| match c.op {
            Op::Insert { gt_idx } | Op::Replace { gt_idx, .. } => Some(gt_idx),
            _ => None,
        })
        .collect();

    // Equal/Fuzzy ops *inside* a boundary region (rare — most matches
    // get pulled into Core) are still real matches; transfer the
    // timestamps directly so we don't lose them.
    for c in region {
        match c.op {
            Op::Equal { whisper_idx, gt_idx } | Op::Fuzzy { whisper_idx, gt_idx, .. } => {
                let w = &whisper[whisper_idx];
                let g = &ground_truth[gt_idx];
                aligned.push(AlignedWord {
                    text: g.text.clone(),
                    start_seconds: w.start_seconds,
                    end_seconds: w.end_seconds,
                    confidence: if matches!(c.op, Op::Equal { .. }) {
                        Confidence::Exact
                    } else {
                        Confidence::Fuzzy
                    },
                });
            }
            _ => {}
        }
    }

    if gt_only.is_empty() {
        return;
    }

    let gt_preview = preview_from_ground_truth(ground_truth, &gt_only);
    trim_log.push(TrimEvent {
        kind: match end {
            BoundaryEnd::Leading => TrimKind::LeadingGroundTruth,
            BoundaryEnd::Trailing => TrimKind::TrailingGroundTruth,
        },
        word_count: gt_only.len(),
        preview: gt_preview,
    });

    match strategy {
        BoundaryStrategy::Drop => {
            // Words excluded entirely; nothing to push.
        }
        BoundaryStrategy::NoSync => {
            for &g_idx in &gt_only {
                aligned.push(AlignedWord {
                    text: ground_truth[g_idx].text.clone(),
                    start_seconds: 0.0,
                    end_seconds: 0.0,
                    confidence: Confidence::Unsynced,
                });
            }
        }
        BoundaryStrategy::Bracket => {
            // Distribute the gap [gap_start, gap_end] proportionally
            // by character count. If the gap is non-positive, fall
            // back to Unsynced.
            let total_chars: usize = gt_only
                .iter()
                .map(|&g| ground_truth[g].text.chars().count().max(1))
                .sum();
            let gap_duration = (gap_end - gap_start).max(0.0);
            if gap_duration <= 0.0 || total_chars == 0 {
                for &g_idx in &gt_only {
                    aligned.push(AlignedWord {
                        text: ground_truth[g_idx].text.clone(),
                        start_seconds: 0.0,
                        end_seconds: 0.0,
                        confidence: Confidence::Unsynced,
                    });
                }
                return;
            }
            let mut cursor = gap_start;
            #[allow(clippy::cast_precision_loss)]
            let per_char = gap_duration / total_chars as f64;
            for &g_idx in &gt_only {
                let chars = ground_truth[g_idx].text.chars().count().max(1);
                #[allow(clippy::cast_precision_loss)]
                let dur = chars as f64 * per_char;
                aligned.push(AlignedWord {
                    text: ground_truth[g_idx].text.clone(),
                    start_seconds: cursor,
                    end_seconds: cursor + dur,
                    confidence: Confidence::Bracketed,
                });
                cursor += dur;
            }
        }
    }
}

/// Walk the core region: Equal/Fuzzy/Replace copy timestamps from
/// Whisper to the GT word at the same position; Delete is discarded
/// (audio time is reclaimed by neighbours via interpolation of any
/// adjacent Inserts); Insert interpolates from neighbours.
fn transfer_core(
    region: &[&ClassifiedOp],
    whisper: &[WordTiming],
    ground_truth: &[GroundTruthWord],
    aligned: &mut Vec<AlignedWord>,
) {
    let mut i = 0;
    while i < region.len() {
        match region[i].op {
            Op::Equal { whisper_idx, gt_idx } => {
                let w = &whisper[whisper_idx];
                aligned.push(AlignedWord {
                    text: ground_truth[gt_idx].text.clone(),
                    start_seconds: w.start_seconds,
                    end_seconds: w.end_seconds,
                    confidence: Confidence::Exact,
                });
                i += 1;
            }
            Op::Fuzzy { whisper_idx, gt_idx, .. } => {
                let w = &whisper[whisper_idx];
                aligned.push(AlignedWord {
                    text: ground_truth[gt_idx].text.clone(),
                    start_seconds: w.start_seconds,
                    end_seconds: w.end_seconds,
                    confidence: Confidence::Fuzzy,
                });
                i += 1;
            }
            Op::Replace { whisper_idx, gt_idx, .. } => {
                // Position-paired with low textual similarity, but
                // they're at the same time slot — copy Whisper's
                // span verbatim. This is what makes the karaoke
                // highlight track audio for "2024" → spoken Dutch
                // form, name swaps, and other near-misses below the
                // fuzzy threshold.
                let w = &whisper[whisper_idx];
                aligned.push(AlignedWord {
                    text: ground_truth[gt_idx].text.clone(),
                    start_seconds: w.start_seconds,
                    end_seconds: w.end_seconds,
                    confidence: Confidence::Replaced,
                });
                i += 1;
            }
            Op::Insert { .. } => {
                // Collect a run of consecutive Inserts (and any
                // preceding/following Deletes) to interpolate as a
                // group.
                let group_start = i;
                while i < region.len()
                    && matches!(region[i].op, Op::Insert { .. } | Op::Delete { .. })
                {
                    i += 1;
                }
                let group_end = i;
                interpolate_insert_run(
                    &region[group_start..group_end],
                    whisper,
                    ground_truth,
                    aligned,
                );
            }
            Op::Delete { .. } => {
                // Skip lone Delete (Whisper hallucination). Time is
                // implicitly reclaimed: the next Equal will start
                // wherever Whisper had it, leaving a small audible
                // pause that's consistent with what was actually said.
                i += 1;
            }
        }
    }
}

/// Interpolate timestamps for a run of `Insert` operations (with
/// optional adjacent Deletes that contribute their time span). Uses
/// the previous aligned word's end and the next core match's start
/// as bounds, distributing time proportionally to character length.
fn interpolate_insert_run(
    run: &[&ClassifiedOp],
    whisper: &[WordTiming],
    ground_truth: &[GroundTruthWord],
    aligned: &mut Vec<AlignedWord>,
) {
    // Determine the time bounds.
    let prev_end = aligned.last().map_or(0.0, |w| w.end_seconds);

    // The "next" anchor time is whatever Whisper word the deletes
    // span, or — if none — the previous timestamp + 0 (which collapses
    // to zero-duration words; better than fabricated time).
    let delete_indices: Vec<usize> = run
        .iter()
        .filter_map(|c| match c.op {
            Op::Delete { whisper_idx } => Some(whisper_idx),
            _ => None,
        })
        .collect();
    let bound_end = if let Some(last_d) = delete_indices.last() {
        whisper[*last_d].end_seconds
    } else {
        prev_end // No delete pool — pure insert with no following anchor here.
    };

    let inserts: Vec<usize> = run
        .iter()
        .filter_map(|c| match c.op {
            Op::Insert { gt_idx } => Some(gt_idx),
            _ => None,
        })
        .collect();

    if inserts.is_empty() {
        return;
    }

    let total_chars: usize = inserts
        .iter()
        .map(|&g| ground_truth[g].text.chars().count().max(1))
        .sum();
    let span = (bound_end - prev_end).max(0.0);

    if span <= 0.0 || total_chars == 0 {
        // Zero-duration: emit at prev_end. Reading systems will skip
        // these instantly but the text remains.
        for &g_idx in &inserts {
            aligned.push(AlignedWord {
                text: ground_truth[g_idx].text.clone(),
                start_seconds: prev_end,
                end_seconds: prev_end,
                confidence: Confidence::Interpolated,
            });
        }
        return;
    }

    let mut cursor = prev_end;
    #[allow(clippy::cast_precision_loss)]
    let per_char = span / total_chars as f64;
    for &g_idx in &inserts {
        let chars = ground_truth[g_idx].text.chars().count().max(1);
        #[allow(clippy::cast_precision_loss)]
        let dur = chars as f64 * per_char;
        aligned.push(AlignedWord {
            text: ground_truth[g_idx].text.clone(),
            start_seconds: cursor,
            end_seconds: cursor + dur,
            confidence: Confidence::Interpolated,
        });
        cursor += dur;
    }
}

fn first_core_match_time(core: &[&ClassifiedOp], whisper: &[WordTiming]) -> Option<f64> {
    core.iter().find_map(|c| match c.op {
        Op::Equal { whisper_idx, .. } | Op::Fuzzy { whisper_idx, .. } => {
            Some(whisper[whisper_idx].start_seconds)
        }
        _ => None,
    })
}

fn last_core_match_time(core: &[&ClassifiedOp], whisper: &[WordTiming]) -> Option<f64> {
    core.iter().rev().find_map(|c| match c.op {
        Op::Equal { whisper_idx, .. } | Op::Fuzzy { whisper_idx, .. } => {
            Some(whisper[whisper_idx].end_seconds)
        }
        _ => None,
    })
}

fn preview_from_whisper(whisper: &[WordTiming], indices: &[usize]) -> String {
    let mut out = String::new();
    for &i in indices.iter().take(15) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&whisper[i].text);
        if out.len() > 80 {
            break;
        }
    }
    if indices.len() > 15 {
        out.push_str(" …");
    }
    out
}

fn preview_from_ground_truth(gt: &[GroundTruthWord], indices: &[usize]) -> String {
    let mut out = String::new();
    for &i in indices.iter().take(15) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&gt[i].text);
        if out.len() > 80 {
            break;
        }
    }
    if indices.len() > 15 {
        out.push_str(" …");
    }
    out
}

/// Clamp any timestamp regressions so SMIL emits monotonic clip times.
/// Whisper itself can occasionally produce overlapping word ranges
/// (BPE artefacts) and our interpolation could in theory exceed the
/// next anchor's start when characters dominate. A simple sweep fixes
/// both.
fn enforce_monotonicity(aligned: &mut [AlignedWord]) {
    for i in 1..aligned.len() {
        // Skip Unsynced entries — they hold zeros by design.
        if aligned[i].confidence == Confidence::Unsynced
            || aligned[i - 1].confidence == Confidence::Unsynced
        {
            continue;
        }
        let prev_end = aligned[i - 1].end_seconds;
        if aligned[i].start_seconds < prev_end {
            aligned[i].start_seconds = prev_end;
        }
        if aligned[i].end_seconds < aligned[i].start_seconds {
            aligned[i].end_seconds = aligned[i].start_seconds;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::classify;
    use crate::diff::diff_words;
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

    fn run(
        whisper: &[WordTiming],
        ground_truth: &[GroundTruthWord],
        strategy: BoundaryStrategy,
    ) -> (Vec<AlignedWord>, Vec<TrimEvent>) {
        let script = diff_words(whisper, ground_truth);
        let classified = classify(&script);
        transfer_timestamps(whisper, ground_truth, &classified, strategy)
    }

    #[test]
    fn perfect_match_passes_timestamps_through() {
        let w = vec![ww("hello", 0.0, 0.5), ww("world", 0.5, 1.5)];
        let g = gt(&["hello", "world"]);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::default());
        assert_eq!(aligned.len(), 2);
        assert_eq!(aligned[0].start_seconds, 0.0);
        assert_eq!(aligned[0].end_seconds, 0.5);
        assert_eq!(aligned[1].start_seconds, 0.5);
        assert_eq!(aligned[1].end_seconds, 1.5);
    }

    #[test]
    fn whisper_preamble_is_trimmed_no_time_smear() {
        // 8 fake preamble words + 5 real matches.
        let mut w: Vec<WordTiming> = (0..8)
            .map(|i| ww(&format!("p{i}"), i as f64, i as f64 + 1.0))
            .collect();
        for i in 0..5 {
            let t = 8.0 + i as f64;
            w.push(ww(&format!("real{i}"), t, t + 1.0));
        }
        let g = gt(&["real0", "real1", "real2", "real3", "real4"]);
        let (aligned, trim_log) = run(&w, &g, BoundaryStrategy::default());
        // The 8 preamble words must NOT bleed into the first real word.
        assert_eq!(aligned.len(), 5);
        assert_eq!(aligned[0].start_seconds, 8.0);
        // Trim log records the preamble.
        assert!(trim_log.iter().any(|e| e.kind == TrimKind::LeadingWhisper && e.word_count == 8));
    }

    #[test]
    fn colophon_no_sync_default() {
        // 5 real matches + 5 colophon words only in ground truth.
        let mut w: Vec<WordTiming> = Vec::new();
        for i in 0..5 {
            let t = i as f64;
            w.push(ww(&format!("real{i}"), t, t + 1.0));
        }
        let mut g_words: Vec<&str> = vec!["real0", "real1", "real2", "real3", "real4"];
        let colophon = ["isbn", "9780000000000", "copyright", "2024", "publisher"];
        g_words.extend(colophon.iter());
        let g = gt(&g_words);
        let (aligned, trim_log) = run(&w, &g, BoundaryStrategy::NoSync);
        assert_eq!(aligned.len(), 10);
        // Last 5 should be Unsynced.
        for w in &aligned[5..] {
            assert_eq!(w.confidence, Confidence::Unsynced);
            assert_eq!(w.start_seconds, 0.0);
            assert_eq!(w.end_seconds, 0.0);
        }
        assert!(trim_log
            .iter()
            .any(|e| e.kind == TrimKind::TrailingGroundTruth && e.word_count == 5));
    }

    #[test]
    fn colophon_drop_excludes_words() {
        let mut w: Vec<WordTiming> = Vec::new();
        for i in 0..5 {
            let t = i as f64;
            w.push(ww(&format!("real{i}"), t, t + 1.0));
        }
        let mut g_words: Vec<&str> = vec!["real0", "real1", "real2", "real3", "real4"];
        g_words.extend(["isbn", "9780000000000"].iter());
        let g = gt(&g_words);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::Drop);
        assert_eq!(aligned.len(), 5);
    }

    #[test]
    fn colophon_bracket_spans_gap() {
        // 5 matches end at t=5.0, total audio extends to t=10.0,
        // bracket strategy should distribute 5s across the colophon.
        let mut w: Vec<WordTiming> = Vec::new();
        for i in 0..5 {
            let t = i as f64;
            w.push(ww(&format!("real{i}"), t, t + 1.0));
        }
        // Add one more whisper word to extend the audio range.
        w.push(ww("trailing-noise", 5.0, 10.0));
        let mut g_words: Vec<&str> = vec!["real0", "real1", "real2", "real3", "real4"];
        g_words.extend(["isbn", "page", "number"].iter());
        let g = gt(&g_words);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::Bracket);
        // Bracketed entries must have non-zero durations and be in
        // [5.0, 10.0].
        let bracketed: Vec<_> = aligned
            .iter()
            .filter(|w| w.confidence == Confidence::Bracketed)
            .collect();
        assert!(!bracketed.is_empty());
        for w in &bracketed {
            assert!(w.start_seconds >= 5.0);
            assert!(w.end_seconds <= 10.0 + 0.01);
            assert!(w.end_seconds > w.start_seconds);
        }
    }

    #[test]
    fn fuzzy_match_transfers_timestamp() {
        let w = vec![ww("antwerpe", 1.0, 2.0)];
        let g = gt(&["antwerpen"]);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::default());
        assert_eq!(aligned.len(), 1);
        assert_eq!(aligned[0].text, "antwerpen");
        assert_eq!(aligned[0].confidence, Confidence::Fuzzy);
        assert_eq!(aligned[0].start_seconds, 1.0);
    }

    #[test]
    fn replace_op_transfers_whisper_timestamp_to_gt_word() {
        // Whisper transcribed "kavija" but the book has "Cavia".
        // Jaro-Winkler is below 0.85 (k vs C, i vs a swap-ish), so
        // the diff emits a Replace op rather than Fuzzy. The GT word
        // must still get a timestamp — Whisper's audio span at that
        // position is the right answer.
        let w = vec![
            ww("hello", 0.0, 1.0),
            ww("kavija", 1.0, 2.5),
            ww("world", 2.5, 3.5),
        ];
        let g = gt(&["hello", "Cavia", "world"]);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::default());
        assert_eq!(aligned.len(), 3);
        assert_eq!(aligned[1].text, "Cavia");
        // The middle word must have *Whisper's* timestamps, not 0–0
        // and not interpolated.
        assert_eq!(aligned[1].start_seconds, 1.0);
        assert_eq!(aligned[1].end_seconds, 2.5);
        assert_eq!(aligned[1].confidence, Confidence::Replaced);
    }

    #[test]
    fn missing_word_interpolates_between_neighbours() {
        // Whisper missed "the": "hello world" vs "hello the world".
        let w = vec![
            ww("hello", 0.0, 1.0),
            ww("world", 2.0, 3.0),
        ];
        let g = gt(&["hello", "the", "world"]);
        // No anchor here (only 2 matches). The classify() falls back
        // to Core for everything, so interpolation runs on the full
        // script.
        let (aligned, _) = run(&w, &g, BoundaryStrategy::default());
        assert_eq!(aligned.len(), 3);
        assert_eq!(aligned[1].text, "the");
        assert_eq!(aligned[1].confidence, Confidence::Interpolated);
        // "the" should be sandwiched between hello.end (1.0) and
        // world.start (2.0).
        assert!(aligned[1].start_seconds >= 1.0);
        assert!(aligned[1].end_seconds <= 2.0 + 0.01);
    }

    #[test]
    fn timestamps_remain_monotonic() {
        let w = vec![
            ww("a", 0.0, 1.0),
            ww("b", 2.0, 3.0),
            ww("c", 4.0, 5.0),
        ];
        let g = gt(&["a", "missing", "b", "c"]);
        let (aligned, _) = run(&w, &g, BoundaryStrategy::default());
        for i in 1..aligned.len() {
            if aligned[i].confidence == Confidence::Unsynced
                || aligned[i - 1].confidence == Confidence::Unsynced
            {
                continue;
            }
            assert!(
                aligned[i].start_seconds >= aligned[i - 1].end_seconds - 1e-9,
                "non-monotonic at {i}: {:?} → {:?}",
                aligned[i - 1],
                aligned[i],
            );
        }
    }
}
