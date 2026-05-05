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

pub use error::{Error, Result};

use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// One transcribed time-range with the text Whisper produced for it.
///
/// Times are in seconds (whisper.cpp returns centiseconds; we convert).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Segment {
    pub start_seconds: f64,
    pub end_seconds: f64,
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

/// Transcribe a single audio file to a flat list of timed text segments.
///
/// Pipeline:
///
/// 1. Decode `audio_path` to floating-point PCM with symphonia.
/// 2. Resample to 16 kHz mono (Whisper's required input format) with rubato.
/// 3. Run whisper.cpp full inference and walk the returned segments.
///
/// Returns segments in chronological order.
pub fn transcribe(audio_path: &Path, options: &TranscribeOptions) -> Result<Vec<Segment>> {
    let samples = decode::decode_to_mono_16khz(audio_path)?;

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

    let mut state = ctx.create_state().map_err(Error::Whisper)?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(&options.language));
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

        // whisper.cpp returns time in centiseconds (10 ms units).
        #[allow(clippy::cast_precision_loss)]
        let start = (t0 as f64) / 100.0;
        #[allow(clippy::cast_precision_loss)]
        let end = (t1 as f64) / 100.0;
        out.push(Segment {
            start_seconds: start,
            end_seconds: end,
            text,
        });
    }
    Ok(out)
}
