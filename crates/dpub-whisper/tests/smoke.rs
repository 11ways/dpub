//! Whisper end-to-end smoke test.
//!
//! Skipped unless both:
//!
//! - `DPUB_TEST_WHISPER_MODEL=/path/to/ggml-*.bin` is set, and
//! - `DPUB_TEST_AUDIO=/path/to/something.mp3` is set
//!
//! Asserts that `transcribe()` returns at least one segment without panicking.
//! Doesn't try to assert the *content* of the transcription — that is
//! model- and language-dependent and easily flaky.

use std::path::PathBuf;

use dpub_whisper::{TranscribeOptions, transcribe};

fn opt(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[test]
fn transcribes_when_model_and_audio_are_provided() {
    let Some(model_path) = opt("DPUB_TEST_WHISPER_MODEL") else {
        eprintln!("DPUB_TEST_WHISPER_MODEL not set — skipping");
        return;
    };
    let Some(audio_path) = opt("DPUB_TEST_AUDIO") else {
        eprintln!("DPUB_TEST_AUDIO not set — skipping");
        return;
    };

    let opts = TranscribeOptions {
        model_path,
        language: std::env::var("DPUB_TEST_WHISPER_LANG").unwrap_or_else(|_| "en".into()),
    };
    let segments = transcribe(&audio_path, &opts).expect("transcribe");
    eprintln!("got {} segments", segments.len());
    for s in segments.iter().take(5) {
        eprintln!(
            "  [{:>6.2}s – {:>6.2}s] {}",
            s.start_seconds, s.end_seconds, s.text
        );
        for w in s.words.iter().take(8) {
            eprintln!(
                "      [{:>6.2}s – {:>6.2}s] {}",
                w.start_seconds, w.end_seconds, w.text
            );
        }
    }
    // Allow zero segments for pure silence/sine input — that's not a bug,
    // it's whisper correctly recognising "no speech".
    // For non-empty segments, the per-word coalescer should always
    // produce at least one word.
    for s in &segments {
        if !s.text.is_empty() {
            assert!(
                !s.words.is_empty(),
                "segment with non-empty text {:?} has empty words",
                s.text,
            );
        }
    }
}
