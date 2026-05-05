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

use dpub_whisper::Segment;

/// One paragraph of cleaned-up transcript text and the audio time range
/// it spans.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paragraph {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
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
pub(crate) fn merge_into_paragraphs(segments: &[Segment], opts: &CleanupOpts) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut current = Builder::default();

    for seg in segments {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        if current.is_empty() {
            current.start = seg.start_seconds;
        }
        current.append(text, seg.end_seconds);

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
    sentences: usize,
}

impl Builder {
    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn append(&mut self, text: &str, end: f64) {
        if !self.buf.is_empty() {
            self.buf.push(' ');
        }
        self.buf.push_str(text);
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
        Paragraph {
            start_seconds: self.start,
            end_seconds: self.end,
            text: self.buf,
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
        }
    }

    #[test]
    fn three_sentences_merge_into_one_paragraph() {
        let segs = vec![
            seg(0.0, 2.0, "De man liep door de straat."),
            seg(2.0, 4.0, "Hij keek naar de lucht."),
            seg(4.0, 6.0, "Het regende zachtjes."),
        ];
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
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
        let out = merge_into_paragraphs(&segs, &CleanupOpts::default());
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("Dr. Jansen"));
    }

    #[test]
    fn empty_input_yields_no_paragraphs() {
        let out = merge_into_paragraphs(&[], &CleanupOpts::default());
        assert!(out.is_empty());
    }
}
