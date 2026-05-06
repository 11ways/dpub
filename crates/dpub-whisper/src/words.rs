//! BPE-token → word coalescer.
//!
//! whisper.cpp emits BPE tokens, not words. `"translate"` comes back as
//! roughly `[" trans", "late"]`; punctuation arrives as its own zero-or-
//! near-zero-duration token; special tokens like `[_BEG_]` and
//! `<|notimestamps|>` are interleaved.
//!
//! This module coalesces a flat token stream back into whole words with
//! sensible audio time ranges, suitable for driving per-word SMIL Media
//! Overlay sync. ASCII-only string ops; deliberately no
//! `unicode-segmentation` dep — the same stance as `text_cleanup`.
//!
//! Word-boundary rule: a token whose text starts with ASCII space is
//! the start of a new word. A token without leading space, or one whose
//! text after trimming is pure ASCII punctuation, attaches to the
//! current word. The first non-special token of the input always starts
//! a new word (defends against whisper occasionally dropping the
//! leading space at segment start).
//!
//! Defensive case: if the previous word ends in a sentence terminator
//! (`. ! ? …` or a closing quote/bracket) and the next token's trimmed
//! text starts with an alphabetic character, treat it as a new word
//! even without a leading space — recovers from whisper occasionally
//! omitting the space after sentence-final punctuation.

use crate::Word;

/// One whisper.cpp token, with its raw centisecond timing as returned
/// by `whisper_full_get_token_data`. Caller is responsible for
/// constructing these from the FFI; the coalescer takes them by slice
/// so it stays unit-testable with literal data.
pub(crate) struct RawToken<'a> {
    pub text: &'a str,
    pub t0_cs: i64,
    pub t1_cs: i64,
}

/// Coalesce a token stream into words.
///
/// `seg_t0_cs` / `seg_t1_cs` are the segment's outer time bounds, also
/// in centiseconds. Word ranges are clamped into `[seg_t0, seg_t1]` so
/// a token whose timing runs slightly past the segment boundary
/// doesn't overlap the next segment's first word.
pub(crate) fn coalesce(tokens: &[RawToken<'_>], seg_t0_cs: i64, seg_t1_cs: i64) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut current: Option<WordBuf> = None;

    for tok in tokens {
        if is_special(tok.text) || tok.text.is_empty() {
            continue;
        }

        let starts_with_space = tok.text.starts_with(' ');
        let trimmed = tok.text.trim_start_matches(' ');
        if trimmed.is_empty() {
            // Pure-whitespace token — drop.
            continue;
        }
        let pure_punct = trimmed.chars().all(is_attaching_punct);

        let starts_new = current.is_none()
            || (starts_with_space && !pure_punct)
            || (!pure_punct
                && trimmed.chars().next().is_some_and(char::is_alphabetic)
                && current.as_ref().is_some_and(WordBuf::ends_in_terminator));

        if starts_new {
            if let Some(w) = current.take() {
                out.push(w.finish());
            }
            current = Some(WordBuf::start(trimmed, tok.t0_cs, tok.t1_cs));
        } else {
            current
                .as_mut()
                .expect("starts_new is false implies current is Some")
                .extend(trimmed, tok.t1_cs);
        }
    }
    if let Some(w) = current {
        out.push(w.finish());
    }

    let seg_lo = cs_to_seconds(seg_t0_cs);
    let seg_hi = cs_to_seconds(seg_t1_cs);
    for w in &mut out {
        if w.start_seconds < seg_lo {
            w.start_seconds = seg_lo;
        }
        if w.end_seconds > seg_hi {
            w.end_seconds = seg_hi;
        }
        if w.end_seconds <= w.start_seconds {
            // Whisper sometimes emits punctuation tokens with t1 == t0.
            // Give the word a 50 ms minimum so reading systems can
            // animate the highlight.
            w.end_seconds = (w.start_seconds + 0.05).min(seg_hi);
        }
    }

    out
}

fn cs_to_seconds(cs: i64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        (cs as f64) / 100.0
    }
}

/// Special whisper.cpp control tokens like `[_BEG_]`, `<|notimestamps|>`,
/// `<|0.00|>`, etc. Heuristic: leading `[` or `<`.
fn is_special(text: &str) -> bool {
    text.starts_with('[') || text.starts_with('<')
}

/// Is `c` an ASCII punctuation char that should attach to the previous
/// word rather than starting a new one? Whitespace is *not* attaching —
/// it lives between word spans in the rendered XHTML.
fn is_attaching_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | '!'
            | '?'
            | ';'
            | ':'
            | '…'
            | ')'
            | ']'
            | '}'
            | '"'
            | '\''
            | '»'
            | '“'
            | '”'
            | '‘'
            | '’'
    )
}

/// Is `c` a sentence-terminal character? Used by the defensive
/// "no-leading-space-after-terminator" branch.
fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…')
}

struct WordBuf {
    text: String,
    t0_cs: i64,
    t1_cs: i64,
}

impl WordBuf {
    fn start(text: &str, t0_cs: i64, t1_cs: i64) -> Self {
        Self {
            text: text.to_owned(),
            t0_cs,
            t1_cs,
        }
    }

    fn extend(&mut self, text: &str, t1_cs: i64) {
        self.text.push_str(text);
        if t1_cs > self.t1_cs {
            self.t1_cs = t1_cs;
        }
    }

    /// Strip trailing closing punctuation and check if the resulting
    /// last char is sentence-terminal. Mirrors the trim done by
    /// `text_cleanup::Builder::ends_at_sentence_terminator`.
    fn ends_in_terminator(&self) -> bool {
        let trimmed = self.text.trim_end_matches(['"', '\'', ')', ']']);
        trimmed
            .chars()
            .next_back()
            .is_some_and(is_terminator)
    }

    fn finish(self) -> Word {
        Word {
            start_seconds: cs_to_seconds(self.t0_cs),
            end_seconds: cs_to_seconds(self.t1_cs),
            text: self.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, t0_cs: i64, t1_cs: i64) -> RawToken<'_> {
        RawToken { text, t0_cs, t1_cs }
    }

    #[test]
    fn coalesces_simple_sentence() {
        // "Het was een test." emitted with leading-space tokens.
        let tokens = vec![
            tok(" Het", 0, 30),
            tok(" was", 30, 60),
            tok(" een", 60, 90),
            tok(" test", 90, 130),
            tok(".", 130, 130),
        ];
        let out = coalesce(&tokens, 0, 200);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].text, "Het");
        assert_eq!(out[1].text, "was");
        assert_eq!(out[2].text, "een");
        assert_eq!(out[3].text, "test.");
        assert!((out[3].start_seconds - 0.90).abs() < 1e-9);
        // The "." token had t0==t1==130; the host word's end is the
        // attached punctuation's t1 (still 130 = 1.30s).
        assert!(out[3].end_seconds >= 1.30);
    }

    #[test]
    fn coalesces_subword_pieces() {
        // " trans" + "late" → "translate" (one word, span first.t0 → last.t1)
        let tokens = vec![tok(" trans", 100, 130), tok("late", 130, 180)];
        let out = coalesce(&tokens, 0, 200);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "translate");
        assert!((out[0].start_seconds - 1.00).abs() < 1e-9);
        assert!((out[0].end_seconds - 1.80).abs() < 1e-9);
    }

    #[test]
    fn drops_special_tokens() {
        let tokens = vec![
            tok("[_BEG_]", 0, 0),
            tok(" hi", 10, 30),
            tok("<|notimestamps|>", 30, 30),
            tok(" there", 30, 60),
        ];
        let out = coalesce(&tokens, 0, 100);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "hi");
        assert_eq!(out[1].text, "there");
    }

    #[test]
    fn clamps_zero_duration_to_50ms_minimum() {
        // A token whose t0 == t1, attached as the only token of a word.
        let tokens = vec![tok(" ok", 50, 50)];
        let out = coalesce(&tokens, 0, 200);
        assert_eq!(out.len(), 1);
        assert!((out[0].start_seconds - 0.50).abs() < 1e-9);
        assert!(
            out[0].end_seconds >= out[0].start_seconds + 0.04,
            "expected ≥50 ms duration, got start={} end={}",
            out[0].start_seconds,
            out[0].end_seconds,
        );
    }

    #[test]
    fn clamps_to_segment_bounds() {
        // Token t1 runs past segment t1; should clamp.
        let tokens = vec![tok(" word", 50, 250)];
        let out = coalesce(&tokens, 0, 200);
        assert_eq!(out.len(), 1);
        assert!(
            (out[0].end_seconds - 2.00).abs() < 1e-9,
            "expected clamp to segment t1 (2.00s), got {}",
            out[0].end_seconds,
        );
    }

    #[test]
    fn handles_no_leading_space_after_punctuation() {
        // " Hi" + "." + "Bye" — the third token has no leading space
        // but the previous word ends in a terminator and the new token
        // is alphabetic, so it should start a new word.
        let tokens = vec![tok(" Hi", 0, 30), tok(".", 30, 30), tok("Bye", 40, 80)];
        let out = coalesce(&tokens, 0, 100);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "Hi.");
        assert_eq!(out[1].text, "Bye");
    }

    #[test]
    fn first_token_without_leading_space_still_starts_a_word() {
        // Whisper sometimes drops the segment-initial space.
        let tokens = vec![tok("Hello", 10, 50), tok(" world", 50, 90)];
        let out = coalesce(&tokens, 0, 100);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "Hello");
        assert_eq!(out[1].text, "world");
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let out = coalesce(&[], 0, 100);
        assert!(out.is_empty());
    }
}
