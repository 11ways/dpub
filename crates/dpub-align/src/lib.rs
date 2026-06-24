//! Align Whisper's approximate word-level transcription against the
//! real book text (ground truth), transferring timestamps so the EPUB
//! ships with accurate prose AND word-level Media Overlay sync.
//!
//! Pipeline:
//!
//! 1. **Section split** — match ground truth headings to NCC headings
//!    so each section's text is identified.
//! 2. **Word diff** — Myers diff with Jaro-Winkler fuzzy promotion
//!    over normalised word keys.
//! 3. **Boundary trim** — discard audiobook preamble / outro and
//!    handle book-only material (colophon etc.) per
//!    [`BoundaryStrategy`].
//! 4. **Timestamp transfer** — copy/redistribute/interpolate Whisper
//!    timings onto the ground truth word stream.
//! 5. **Paragraph reconstruction** — group aligned words by ground
//!    truth paragraph breaks (blank lines) and emit
//!    [`AlignedParagraph`].

mod boundary;
mod diff;
mod error;
mod json_format;
mod normalize;
mod section_split;
mod transfer;

pub use error::{Error, Result};
pub use section_split::SectionText;

/// Split a ground truth file into per-section chunks matching the
/// supplied NCC headings.
///
/// Auto-detects format:
/// - **JSON** (first non-whitespace char is `{`): parsed as the
///   structured chapter array (see [`json_format`]) and converted to
///   markdown internally.
/// - **Markdown / plain text**: passed through to the heading-line
///   splitter as-is.
pub fn split_into_sections(text: &str, ncc_headings: &[(&str, usize)]) -> Vec<SectionText> {
    if json_format::looks_like_json(text) {
        let markdown = json_format::convert_to_markdown(text);
        section_split::split_into_sections(&markdown, ncc_headings)
    } else {
        section_split::split_into_sections(text, ncc_headings)
    }
}

/// Lightweight word-with-timestamp type. Mirrors `dpub_whisper::Word`
/// but keeps `dpub-align` free of whisper.cpp build dependencies.
#[derive(Debug, Clone, PartialEq)]
pub struct WordTiming {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// One ground truth word with the timestamp transferred from Whisper
/// (or interpolated when Whisper had no match).
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedWord {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub confidence: Confidence,
}

/// Provenance of an aligned word's timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Direct match — Whisper's normalised key equals the ground truth's.
    Exact,
    /// Near-match (Jaro-Winkler ≥ 0.85).
    Fuzzy,
    /// Position-paired but not similar enough to be a fuzzy match —
    /// timestamp comes from Whisper's audio span at the same position;
    /// text comes from the ground truth.
    Replaced,
    /// Inserted; timestamp interpolated proportionally from neighbours.
    Interpolated,
    /// Outside the anchor region under `bracket` strategy: timestamp
    /// spans a slice of the leading/trailing gap.
    Bracketed,
    /// Outside the anchor region under `no-sync` strategy: word has
    /// text but no usable timestamp (caller should omit from SMIL).
    Unsynced,
}

/// One paragraph's worth of aligned words, ready to feed into the
/// existing XHTML/SMIL pipeline.
#[derive(Debug, Clone)]
pub struct AlignedParagraph {
    pub text: String,
    pub words: Vec<AlignedWord>,
    pub audio_src: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// How to handle ground-truth-only words (book content the narrator
/// skipped — colophon, index, acknowledgements).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundaryStrategy {
    /// Drop the words from the EPUB entirely.
    Drop,
    /// Include the text but emit no Media Overlay entry — visible in
    /// the XHTML, no karaoke highlight on those passages. Default
    /// because for accessibility readable text matters more than
    /// perfect highlight tracking.
    #[default]
    NoSync,
    /// Span the available time gap proportionally — highlight bar
    /// moves through the words at average speed. Produces continuous
    /// sync at the cost of timestamp accuracy.
    Bracket,
}

/// Diagnostic for one trimmed/dropped/bracketed region.
#[derive(Debug, Clone)]
pub struct TrimEvent {
    pub kind: TrimKind,
    pub word_count: usize,
    /// First ~80 chars of the trimmed text, for log lines.
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimKind {
    /// Whisper-only words before the leading anchor (audiobook preamble).
    LeadingWhisper,
    /// Whisper-only words after the trailing anchor (audiobook outro).
    TrailingWhisper,
    /// Ground-truth-only words before the leading anchor.
    LeadingGroundTruth,
    /// Ground-truth-only words after the trailing anchor.
    TrailingGroundTruth,
}

/// Result of aligning one section: paragraphs to embed in the EPUB,
/// plus a log of every trim/drop event for the user to inspect.
#[derive(Debug, Clone)]
pub struct AlignmentResult {
    pub paragraphs: Vec<AlignedParagraph>,
    pub trim_log: Vec<TrimEvent>,
}

/// Align ground truth text against a Whisper word stream for one section.
///
/// `whisper_words` is the flat list of Whisper words for this section's
/// audio (in chronological order). `ground_truth` is the section's text
/// with paragraph boundaries marked by blank lines. `audio_src` is the
/// audio filename (basename) that all produced paragraphs will inherit.
pub fn align_section(
    whisper_words: &[WordTiming],
    ground_truth: &str,
    audio_src: &str,
    boundary_strategy: BoundaryStrategy,
) -> Result<AlignmentResult> {
    if whisper_words.is_empty() {
        return Err(Error::NoWhisperWords);
    }
    if ground_truth.trim().is_empty() {
        return Err(Error::EmptyGroundTruth);
    }

    // Tokenise ground truth into paragraphs of words. Each word
    // remembers the paragraph index it belongs to, so we can rebuild
    // paragraph structure after timestamp transfer.
    let (gt_words, paragraph_breaks) = tokenise_ground_truth(ground_truth);

    // Run word-level diff on normalised keys.
    let edit_script = diff::diff_words(whisper_words, &gt_words);

    // Detect the alignment anchor region and classify ops as
    // leading/core/trailing.
    let trimmed = boundary::classify(&edit_script);

    // Walk the classified edit script and produce one AlignedWord per
    // ground truth word.
    let (aligned, trim_log) = transfer::transfer_timestamps(
        whisper_words,
        &gt_words,
        &trimmed,
        boundary_strategy,
    );

    // Group aligned words by paragraph (using the breaks we recorded).
    let paragraphs = build_paragraphs(&aligned, &paragraph_breaks, audio_src);

    Ok(AlignmentResult {
        paragraphs,
        trim_log,
    })
}

/// Internal: one ground truth word with original surface text and a
/// match key (normalised form for diffing).
#[derive(Debug, Clone)]
pub(crate) struct GroundTruthWord {
    pub text: String,
    pub key: String,
}

fn tokenise_ground_truth(text: &str) -> (Vec<GroundTruthWord>, Vec<usize>) {
    let mut words: Vec<GroundTruthWord> = Vec::new();
    let mut paragraph_breaks: Vec<usize> = Vec::new();

    for (para_idx, para) in text.split("\n\n").enumerate() {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if para_idx > 0 && !words.is_empty() {
            paragraph_breaks.push(words.len());
        }
        for tok in para.split_whitespace() {
            let key = normalize::normalise(tok);
            if key.is_empty() {
                // Pure punctuation token — attach to the previous word
                // by appending raw text, leaving its key untouched.
                if let Some(last) = words.last_mut() {
                    last.text.push_str(tok);
                    continue;
                }
            }
            words.push(GroundTruthWord {
                text: tok.to_owned(),
                key,
            });
        }
    }

    (words, paragraph_breaks)
}

fn build_paragraphs(
    aligned: &[AlignedWord],
    paragraph_breaks: &[usize],
    audio_src: &str,
) -> Vec<AlignedParagraph> {
    if aligned.is_empty() {
        return Vec::new();
    }
    let mut paragraphs = Vec::new();
    let mut start_idx = 0;
    let mut breaks: Vec<usize> = paragraph_breaks.to_vec();
    breaks.push(aligned.len()); // sentinel

    for end_idx in breaks {
        if end_idx <= start_idx {
            continue;
        }
        let slice = &aligned[start_idx..end_idx];
        if slice.is_empty() {
            start_idx = end_idx;
            continue;
        }
        let text = slice
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        // Time bounds: pick the first/last word with a real timestamp
        // (Unsynced words may have zeros).
        let first_real = slice.iter().find(|w| w.confidence != Confidence::Unsynced);
        let last_real = slice
            .iter()
            .rev()
            .find(|w| w.confidence != Confidence::Unsynced);
        let (start_seconds, end_seconds) = match (first_real, last_real) {
            (Some(a), Some(b)) => (a.start_seconds, b.end_seconds),
            _ => (0.0, 0.0),
        };
        paragraphs.push(AlignedParagraph {
            text,
            words: slice.to_vec(),
            audio_src: audio_src.to_owned(),
            start_seconds,
            end_seconds,
        });
        start_idx = end_idx;
    }
    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ww(text: &str, start: f64, end: f64) -> WordTiming {
        WordTiming {
            start_seconds: start,
            end_seconds: end,
            text: text.to_owned(),
        }
    }

    #[test]
    fn rejects_empty_inputs() {
        assert!(matches!(
            align_section(&[], "hello", "a.mp3", BoundaryStrategy::default()),
            Err(Error::NoWhisperWords),
        ));
        assert!(matches!(
            align_section(&[ww("a", 0.0, 1.0)], "", "a.mp3", BoundaryStrategy::default()),
            Err(Error::EmptyGroundTruth),
        ));
    }

    #[test]
    fn perfect_match_passes_through() {
        // Whisper says exactly what the ground truth says.
        let whisper = vec![
            ww("Hello", 0.0, 0.5),
            ww("world.", 0.5, 1.5),
        ];
        let gt = "Hello world.";
        let res = align_section(&whisper, gt, "a.mp3", BoundaryStrategy::default()).unwrap();
        assert_eq!(res.paragraphs.len(), 1);
        let para = &res.paragraphs[0];
        assert_eq!(para.words.len(), 2);
        assert_eq!(para.words[0].text, "Hello");
        assert_eq!(para.words[1].text, "world.");
        assert_eq!(para.words[0].confidence, Confidence::Exact);
        assert_eq!(para.start_seconds, 0.0);
        assert_eq!(para.end_seconds, 1.5);
        assert!(res.trim_log.is_empty());
    }

    #[test]
    fn paragraph_breaks_split_output() {
        let whisper = vec![
            ww("Hello", 0.0, 0.5),
            ww("world.", 0.5, 1.5),
            ww("Foo", 2.0, 2.4),
            ww("bar.", 2.4, 3.0),
        ];
        let gt = "Hello world.\n\nFoo bar.";
        let res = align_section(&whisper, gt, "a.mp3", BoundaryStrategy::default()).unwrap();
        assert_eq!(res.paragraphs.len(), 2);
        assert_eq!(res.paragraphs[0].text, "Hello world.");
        assert_eq!(res.paragraphs[1].text, "Foo bar.");
    }
}
