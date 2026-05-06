//! Post-processing for Whisper output.
//!
//! Whisper returns ~10–30 s segments driven by its own chunking strategy,
//! not by sentence or paragraph structure. Emitting one `<p>` per segment
//! gives unreadable prose: paragraphs every twenty seconds, often starting
//! mid-sentence. This module merges segments into paragraphs of roughly
//! 3–6 sentences each while preserving the union timing span.
//!
//! The merged-paragraph `[start, end]` interval is the union of its
//! constituents (`first.start`, `last.end`). Whisper segments are
//! non-overlapping and chronological, so this is the trivially-correct
//! span for any future per-paragraph Media Overlay sync.

use dpub_whisper::{Segment, Word};

/// One paragraph of cleaned-up transcript text and the audio time range
/// it spans.
///
/// `words` carries per-word timings for SMIL Media Overlay sync. It is
/// non-empty whenever `text` is non-empty, *provided* the input
/// segments came from a real Whisper run (the test helper builds
/// segments without per-word data, in which case `words` is empty —
/// callers using it for SMIL emission should fall back gracefully).
///
/// `audio_src` is the basename of the audio file these words came from
/// (e.g. `"07_Inleiding.mp3"`). Invariant: every word in a paragraph
/// comes from the same audio file. The cleanup state machine doesn't
/// merge across audio-file boundaries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paragraph {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub words: Vec<Word>,
    pub audio_src: String,
}

/// Tunable bounds for [`merge_into_paragraphs`]. See module docs for the
/// algorithm; defaults target prose-shaped paragraphs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CleanupOpts {
    pub min_sentences: usize,
    pub max_sentences: usize,
    pub min_chars: usize,
    pub max_chars: usize,
}

impl Default for CleanupOpts {
    fn default() -> Self {
        Self {
            min_sentences: 3,
            max_sentences: 6,
            min_chars: 300,
            max_chars: 600,
        }
    }
}

/// Merge a chronological segment stream into prose-shaped paragraphs.
///
/// Greedy single-pass state machine: append segments until we hit a
/// sentence terminator AND the paragraph has accumulated enough sentences
/// or characters; force-flush at the upper caps so a hallucinated run
/// without punctuation can't grow unbounded.
///
/// `audio_srcs` is a parallel slice giving the audio basename for each
/// segment. The cleanup state machine flushes the current paragraph
/// when the audio file changes, preserving the invariant that every
/// `Paragraph.words[i]` came from the same audio file.
pub(crate) fn merge_into_paragraphs(
    segments: &[Segment],
    audio_srcs: &[String],
    opts: &CleanupOpts,
) -> Vec<Paragraph> {
    debug_assert_eq!(
        segments.len(),
        audio_srcs.len(),
        "segments and audio_srcs must be parallel slices",
    );

    let mut out = Vec::new();
    let mut current = Builder::default();

    for (seg, audio_src) in segments.iter().zip(audio_srcs.iter()) {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        // Audio-file boundary forces a flush so the resulting paragraph
        // doesn't span two audio files (would break per-word SMIL since
        // each `<par>` carries one `<audio src=...>`).
        if !current.is_empty() && current.audio_src != *audio_src {
            out.push(current.finalize());
            current = Builder::default();
        }
        if current.is_empty() {
            current.start = seg.start_seconds;
            current.audio_src.clone_from(audio_src);
        }
        current.append(text, &seg.words, seg.end_seconds);

        let terminator = current.ends_at_sentence_terminator();
        if terminator {
            current.sentences += 1;
        }

        let big_enough = current.sentences >= opts.min_sentences && current.buf.len() >= opts.min_chars;
        let too_big = current.sentences >= opts.max_sentences || current.buf.len() >= opts.max_chars;

        if (terminator && big_enough) || too_big {
            out.push(current.finalize());
            current = Builder::default();
        }
    }
    if !current.is_empty() {
        out.push(current.finalize());
    }
    out
}

#[derive(Default)]
struct Builder {
    start: f64,
    end: f64,
    buf: String,
    words: Vec<Word>,
    audio_src: String,
    sentences: usize,
}

impl Builder {
    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn append(&mut self, text: &str, words: &[Word], end: f64) {
        if !self.buf.is_empty() {
            self.buf.push(' ');
        }
        self.buf.push_str(text);
        self.words.extend_from_slice(words);
        self.end = end;
    }

    /// Return true if `self.buf` ends at a "real" sentence terminator —
    /// `.`, `!`, `?`, or `…` — but not on a digit-decimal (`3.14`) or
    /// known abbreviation (`Dr.`, `bv.`, `enz.`).
    fn ends_at_sentence_terminator(&self) -> bool {
        let trimmed = self.buf.trim_end_matches(['"', '\'', ')', ']']);
        let Some(last) = trimmed.chars().next_back() else {
            return false;
        };
        if last == '!' || last == '?' || last == '…' {
            return true;
        }
        if last != '.' {
            return false;
        }
        let before_dot = &trimmed[..trimmed.len() - last.len_utf8()];
        if before_dot
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return false;
        }
        let tail_word: String = before_dot
            .chars()
            .rev()
            .take_while(|c| c.is_alphabetic() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let tail_lower = tail_word.to_ascii_lowercase();
        if ABBREVIATIONS.iter().any(|a| tail_lower.ends_with(a)) {
            return false;
        }
        true
    }

    fn finalize(mut self) -> Paragraph {
        capitalise_first(&mut self.buf);
        if let Some(first_word) = self.words.first_mut() {
            capitalise_first(&mut first_word.text);
        }
        Paragraph {
            start_seconds: self.start,
            end_seconds: self.end,
            text: self.buf,
            words: self.words,
            audio_src: self.audio_src,
        }
    }
}

/// Tail-suffix match list. Each entry is the lowercase form of an
/// abbreviation that ends in `.` and must NOT be treated as a sentence
/// terminator. Match is suffix-of-the-trailing-alphabetic-run, so
/// `enz.` matches both `enz.` and `... enz.`. Dutch + a few English.
const ABBREVIATIONS: &[&str] = &[
    "dr", "mr", "mrs", "ms", "drs", "ir", "ing", "prof", "dhr", "mw", "jr", "sr", "st", "nr",
    "blz", "bv", "bijv", "enz", "etc", "i.e", "e.g", "o.a", "nl", "vs", "incl", "excl", "tel",
];

fn capitalise_first(s: &mut String) {
    let first = s.chars().next();
    if let Some(c) = first
        && c.is_lowercase()
    {
        let upper: String = c.to_uppercase().collect();
        s.replace_range(..c.len_utf8(), &upper);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start_seconds: start,
            end_seconds: end,
            text: text.into(),
            words: Vec::new(),
        }
    }

    /// Test helper: build a parallel `audio_srcs` slice that pairs every
    /// segment with the same audio file name. Most cleanup tests don't
    /// care about audio-file boundaries; the dedicated test
    /// `audio_file_boundary_forces_flush` exercises the multi-file path.
    fn srcs(n: usize) -> Vec<String> {
        vec!["audio.mp3".to_owned(); n]
    }

    /// Test helper: call `merge_into_paragraphs` against a slice of
    /// segments that all came from the same audio file. Saves every
    /// existing test from threading parallel audio basenames.
    fn merge(segs: &[Segment]) -> Vec<Paragraph> {
        let audio = srcs(segs.len());
        merge_into_paragraphs(segs, &audio, &CleanupOpts::default())
    }

    #[test]
    fn three_sentences_merge_into_one_paragraph() {
        let segs = vec![
            seg(0.0, 2.0, "De man liep door de straat."),
            seg(2.0, 4.0, "Hij keek naar de lucht."),
            seg(4.0, 6.0, "Het regende zachtjes."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("De man liep"));
        assert!(out[0].text.ends_with("zachtjes."));
        assert!((out[0].start_seconds - 0.0).abs() < 1e-9);
        assert!((out[0].end_seconds - 6.0).abs() < 1e-9);
    }

    #[test]
    fn min_chars_holds_short_sentences_together() {
        // Three short sentences below min_chars=300 — should keep merging.
        let segs = vec![
            seg(0.0, 1.0, "Ja."),
            seg(1.0, 2.0, "Nee."),
            seg(2.0, 3.0, "Misschien."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1, "short sentences should not split");
    }

    #[test]
    fn max_sentences_forces_split() {
        // Eight terminated sentences; max_sentences=6 forces a split.
        let segs: Vec<Segment> = (0..8)
            .map(|i| {
                seg(
                    f64::from(i),
                    f64::from(i + 1),
                    "De man liep door de lange koude winterse straat naar huis.",
                )
            })
            .collect();
        let out = merge(&segs);
        assert_eq!(out.len(), 2);
        // First paragraph should hold the first 6 sentences.
        assert_eq!(out[0].text.matches('.').count(), 6);
    }

    #[test]
    fn max_chars_safety_valve_on_long_unpunctuated_run() {
        // A single very long segment with no punctuation must not produce
        // an unbounded paragraph.
        let long = "a ".repeat(400);
        let segs = vec![seg(0.0, 30.0, long.trim())];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.len() <= 800);
    }

    #[test]
    fn open_paragraph_is_flushed_at_end() {
        // Two sentences below min — still gets flushed at end-of-input.
        let segs = vec![
            seg(0.0, 1.0, "Een korte zin."),
            seg(1.0, 2.0, "En nog een."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!((out[0].end_seconds - 2.0).abs() < 1e-9);
    }

    #[test]
    fn decimal_number_does_not_terminate_sentence() {
        // "3.14" should not split. Make sentence #1 long so min_chars is met
        // before the decimal appears, and verify the decimal doesn't add an
        // extra terminator count.
        let segs = vec![
            seg(0.0, 5.0, "Het was lang geleden dat hij iets dergelijks meegemaakt had en hij was er nog niet helemaal klaar voor."),
            seg(5.0, 10.0, "Hij was 3.14 keer ouder dan zij."),
            seg(10.0, 15.0, "Toen vertrok hij naar het volgende dorp."),
        ];
        let out = merge(&segs);
        // Three real sentences (".", ".", ".") in the input; the decimal
        // is not counted, so we get exactly one paragraph.
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("3.14"));
    }

    #[test]
    fn dutch_abbreviation_does_not_terminate_sentence() {
        let segs = vec![
            seg(0.0, 5.0, "In de boekenkast lagen romans, gedichtenbundels, kookboeken enz."),
            seg(5.0, 10.0, "die hij allemaal had gelezen en zorgvuldig had bewaard."),
            seg(10.0, 15.0, "Hij was er trots op."),
        ];
        let out = merge(&segs);
        // "enz." should not be treated as terminator, so this reads as
        // one or two real sentences depending on what does terminate.
        // Specifically, only ". " on "bewaard." and "trots op." count.
        assert!(out[0].text.contains("enz."));
        assert!(out[0].text.contains("bewaard."));
    }

    #[test]
    fn first_letter_is_capitalised() {
        let segs = vec![seg(
            0.0,
            5.0,
            "and then he came back home after a long day at the office.",
        )];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("And then"));
    }

    #[test]
    fn empty_segments_are_skipped() {
        let segs = vec![
            seg(0.0, 1.0, ""),
            seg(1.0, 2.0, "   "),
            seg(2.0, 3.0, "Hallo wereld."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!((out[0].start_seconds - 2.0).abs() < 1e-9);
    }

    #[test]
    fn timing_spans_first_to_last_segment() {
        let segs = vec![
            seg(10.5, 12.0, "De man liep door de straat naar het einde van de wereld."),
            seg(12.0, 15.0, "Het was een lange weg."),
            seg(15.0, 18.7, "Maar hij gaf niet op."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!((out[0].start_seconds - 10.5).abs() < 1e-9);
        assert!((out[0].end_seconds - 18.7).abs() < 1e-9);
    }

    #[test]
    fn dr_does_not_terminate_sentence() {
        let segs = vec![
            seg(0.0, 5.0, "De vergadering was gepland in de ruime conferentiezaal aan de straatkant."),
            seg(5.0, 10.0, "Dr. Jansen kwam binnen en groette iedereen vriendelijk."),
            seg(10.0, 15.0, "Hij ging zitten en de zitting begon."),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("Dr. Jansen"));
    }

    #[test]
    fn empty_input_yields_no_paragraphs() {
        let out = merge_into_paragraphs(&[], &[], &CleanupOpts::default());
        assert!(out.is_empty());
    }

    fn word(start: f64, end: f64, text: &str) -> Word {
        Word {
            start_seconds: start,
            end_seconds: end,
            text: text.into(),
        }
    }

    fn seg_with_words(start: f64, end: f64, text: &str, words: Vec<Word>) -> Segment {
        Segment {
            start_seconds: start,
            end_seconds: end,
            text: text.into(),
            words,
        }
    }

    #[test]
    fn words_thread_through_to_paragraph() {
        // Two segments with synthetic per-word data; merged paragraph
        // should preserve every word in document order.
        let segs = vec![
            seg_with_words(
                0.0,
                1.5,
                "Hallo wereld.",
                vec![word(0.0, 0.5, "Hallo"), word(0.5, 1.5, "wereld.")],
            ),
            seg_with_words(
                1.5,
                3.0,
                "Goedemorgen.",
                vec![word(1.5, 3.0, "Goedemorgen.")],
            ),
        ];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].words.len(), 3);
        assert_eq!(out[0].words[0].text, "Hallo");
        assert_eq!(out[0].words[1].text, "wereld.");
        assert_eq!(out[0].words[2].text, "Goedemorgen.");
        assert_eq!(out[0].audio_src, "audio.mp3");
    }

    #[test]
    fn capitalisation_propagates_to_first_word() {
        // Paragraph starts mid-sentence with a lowercase word; the
        // capitalisation fix must update both the rendered text AND
        // the first word's text so the visible <span> reads "And".
        let segs = vec![seg_with_words(
            0.0,
            2.0,
            "and then he ran.",
            vec![
                word(0.0, 0.3, "and"),
                word(0.3, 0.6, "then"),
                word(0.6, 0.9, "he"),
                word(0.9, 2.0, "ran."),
            ],
        )];
        let out = merge(&segs);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("And"));
        assert_eq!(out[0].words[0].text, "And");
    }

    #[test]
    fn audio_file_boundary_forces_flush() {
        // Two segments from different audio files. Even mid-sentence,
        // the cleanup must flush at the file boundary so each
        // resulting paragraph references one audio file.
        let segs = vec![
            seg(0.0, 1.0, "Eerste deel."),
            seg(0.0, 1.0, "Tweede deel."),
        ];
        let audio = vec!["a.mp3".to_owned(), "b.mp3".to_owned()];
        let out = merge_into_paragraphs(&segs, &audio, &CleanupOpts::default());
        assert_eq!(out.len(), 2, "audio-file boundary must flush");
        assert_eq!(out[0].audio_src, "a.mp3");
        assert_eq!(out[1].audio_src, "b.mp3");
    }
}
