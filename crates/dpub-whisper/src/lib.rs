//! Local Whisper transcription, used by [dpub-convert] to fill in the
//! text layer of audio-only DAISY 2.02 books before EPUB 3 assembly.
//!
//! Wraps [`whisper-rs`] (FFI to whisper.cpp) and decodes audio with
//! [`symphonia`] + [`rubato`] so callers don't need ffmpeg.
//!
//! ## Quickstart
//!
//! ```no_run
//! use dpub_whisper::{transcribe, TranscribeOptions};
//! # let opts = TranscribeOptions {
//! #     model_path: "ggml-medium.bin".into(),
//! #     language: "nl".into(),
//! # };
//! let segments = transcribe("audio.mp3".as_ref(), &opts)?;
//! for seg in &segments {
//!     println!("[{:>7.3} – {:>7.3}] {}", seg.start_seconds, seg.end_seconds, seg.text);
//! }
//! # Ok::<_, dpub_whisper::Error>(())
//! ```
//!
//! ## Models
//!
//! Whisper.cpp uses GGML-format model files. Download from
//! <https://huggingface.co/ggerganov/whisper.cpp/tree/main>. Recommended:
//!
//! - `ggml-base.bin`   ≈ 142 MB — fast, lower quality
//! - `ggml-small.bin`  ≈ 466 MB — good general-purpose default
//! - `ggml-medium.bin` ≈ 515 MB — higher quality for production books
//! - `ggml-large-v3.bin` ≈ 1.0 GB — best quality, slowest
//!
//! [dpub-convert]: https://docs.rs/dpub-convert
//! [`whisper-rs`]: https://docs.rs/whisper-rs
//! [`symphonia`]: https://docs.rs/symphonia
//! [`rubato`]: https://docs.rs/rubato

mod decode;
mod error;
mod words;

pub use error::{Error, Result};

use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// One transcribed time-range with the text Whisper produced for it.
///
/// Times are in seconds (whisper.cpp returns centiseconds; we convert).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Segment {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    /// Per-word timings derived from whisper.cpp's per-token data, with
    /// BPE subword pieces coalesced back into whole words. Empty when
    /// `text` is empty; otherwise one entry per visible word in the
    /// segment, in chronological order.
    pub words: Vec<Word>,
}

/// One transcribed word with its audio time range.
///
/// Used to drive per-word SMIL Media Overlay sync (`<par>` per word in
/// the produced EPUB). Times are in seconds; whisper.cpp's token
/// timestamps are notoriously approximate (~100–300 ms tolerance), so
/// callers should not rely on word boundaries being lip-sync-accurate.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Word {
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// The visible word text, with no leading whitespace. Trailing
    /// punctuation that the whisper tokenizer emitted as a separate
    /// token is attached here (e.g. `"wereld."`).
    pub text: String,
}

/// Knobs for [`transcribe`].
#[derive(Debug, Clone)]
pub struct TranscribeOptions {
    /// Path to a `ggml-*.bin` model file. See module docs for download links.
    pub model_path: PathBuf,
    /// ISO 639-1 language code (e.g. `"nl"`, `"en"`, `"de"`). Whisper's
    /// `"auto"` is supported but not recommended for audiobooks where the
    /// language is known up-front.
    pub language: String,
}

/// Owns a loaded Whisper model and lets you transcribe many audio files
/// against it without re-loading the GGML weights every time.
///
/// Loading a medium-size GGML model (~1.5 GB) takes several seconds and
/// allocates the same amount on the GPU when built with `metal` /
/// `cuda`. A typical talking book has 30+ audio files; constructing one
/// `Transcriber` and reusing it across the whole book amortises that
/// cost. Each [`Transcriber::transcribe`] call still creates a fresh
/// decoder state internally, so per-file decoding stays independent.
pub struct Transcriber {
    ctx: WhisperContext,
    language: String,
}

impl Transcriber {
    /// Load a GGML model and bind it to a target language. The expensive
    /// part — the file → buffer → GPU copy — happens here, once.
    pub fn new(options: &TranscribeOptions) -> Result<Self> {
        let ctx = WhisperContext::new_with_params(
            options
                .model_path
                .to_str()
                .ok_or_else(|| Error::InvalidModelPath(options.model_path.clone()))?,
            WhisperContextParameters::default(),
        )
        .map_err(|source| Error::ModelLoad {
            path: options.model_path.clone(),
            source,
        })?;
        Ok(Self {
            ctx,
            language: options.language.clone(),
        })
    }

    /// Transcribe one audio file to a flat list of timed text segments.
    ///
    /// Pipeline:
    ///
    /// 1. Decode `audio_path` to floating-point PCM with symphonia.
    /// 2. Resample to 16 kHz mono (Whisper's required input format) with rubato.
    /// 3. Run whisper.cpp full inference and walk the returned segments.
    ///
    /// Each call gets a fresh `WhisperState` so decoder caches don't
    /// leak between files. Returns segments in chronological order.
    pub fn transcribe(&self, audio_path: &Path) -> Result<Vec<Segment>> {
        let samples = decode::decode_to_mono_16khz(audio_path)?;

        let mut state = self.ctx.create_state().map_err(Error::Whisper)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);

        state.full(params, &samples).map_err(Error::Whisper)?;

        let count = state.full_n_segments();
        #[allow(clippy::cast_sign_loss)]
        let cap = count.max(0) as usize;
        let mut out = Vec::with_capacity(cap);
        for i in 0..count {
            let Some(seg) = state.get_segment(i) else {
                continue;
            };
            let t0 = seg.start_timestamp();
            let t1 = seg.end_timestamp();
            let text = seg
                .to_str_lossy()
                .map_err(Error::Whisper)?
                .trim()
                .to_owned();

            // Walk every token in this segment and collect the raw
            // (text, t0, t1) triples for the coalescer. whisper.cpp
            // emits BPE tokens; the coalescer turns them back into
            // visible words with sensible audio time ranges.
            let n_tokens = seg.n_tokens();
            #[allow(clippy::cast_sign_loss)]
            let tok_cap = n_tokens.max(0) as usize;
            let mut raw_tokens: Vec<words::RawToken<'_>> = Vec::with_capacity(tok_cap);
            for j in 0..n_tokens {
                let Some(tok) = seg.get_token(j) else {
                    continue;
                };
                // Defensive: skip tokens whose text isn't valid UTF-8.
                // Whisper occasionally emits partial multibyte sequences
                // mid-word; we'd rather drop a token than poison the segment.
                let Ok(token_text) = tok.to_str() else {
                    continue;
                };
                let data = tok.token_data();
                raw_tokens.push(words::RawToken {
                    text: token_text,
                    t0_cs: data.t0,
                    t1_cs: data.t1,
                });
            }
            let words_vec = words::coalesce(&raw_tokens, t0, t1);

            // whisper.cpp returns time in centiseconds (10 ms units).
            #[allow(clippy::cast_precision_loss)]
            let start = (t0 as f64) / 100.0;
            #[allow(clippy::cast_precision_loss)]
            let end = (t1 as f64) / 100.0;
            out.push(Segment {
                start_seconds: start,
                end_seconds: end,
                text,
                words: words_vec,
            });
        }
        Ok(out)
    }
}

/// One-shot convenience: build a [`Transcriber`] and use it for a single
/// file. Prefer [`Transcriber::new`] + [`Transcriber::transcribe`] when
/// you have multiple files to process — the model load is the expensive
/// part and you want to amortise it.
pub fn transcribe(audio_path: &Path, options: &TranscribeOptions) -> Result<Vec<Segment>> {
    Transcriber::new(options)?.transcribe(audio_path)
}
