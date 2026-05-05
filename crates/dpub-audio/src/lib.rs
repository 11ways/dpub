//! Audio re-encoding for the dpub toolkit.
//!
//! v1 strategy: shell out to the system [`ffmpeg`](https://ffmpeg.org/)
//! binary. Reasons:
//!
//! - Pure-Rust MP3-decode + Opus-encode + Ogg-mux is a standalone project
//!   (≥ 500 LOC and several FFI dependencies). Worth doing eventually but
//!   not for the v1 ship.
//! - `ffmpeg` is the de-facto standard on every platform and trivially
//!   available (`brew install ffmpeg`, `apt install ffmpeg`, …).
//! - We already shell out to `epubcheck` for validation, so adding one
//!   external dependency does not change the deployment story.
//!
//! When the work moves to pure-Rust later, the public API (just
//! [`recompress_to_opus`]) stays the same and the rest of the crate can
//! swap underneath.

mod error;

pub use error::{Error, Result};

use std::path::{Path, PathBuf};
use std::process::Command;

/// Voice-optimised default bitrate for Opus on speech audio. 64 kbit/s
/// produces audibly transparent speech and roughly 3× compression vs
/// MP3 at typical audiobook bitrates.
pub const DEFAULT_OPUS_BITRATE_KBPS: u32 = 64;

/// `true` if `ffmpeg` is available on `PATH`.
pub fn ffmpeg_available() -> bool {
    which::which("ffmpeg").is_ok()
}

/// Re-encode `input` (any format ffmpeg can decode) into an Ogg/Opus file
/// at `output`.
///
/// `bitrate_kbps` is the target Opus bitrate in kbit/s. The encoder runs
/// in `-application voip` mode, which is tuned for speech and narrow-band
/// content — perfect for audiobooks. For general music, a caller would
/// want `-application audio` and a higher bitrate; that's intentionally
/// not exposed yet because dpub's default workload is talking books.
///
/// Overwrites `output` if it already exists.
pub fn recompress_to_opus(input: &Path, output: &Path, bitrate_kbps: u32) -> Result<()> {
    let bin = which::which("ffmpeg").map_err(|_| Error::FfmpegMissing)?;
    let bitrate = format!("{bitrate_kbps}k");

    let status = Command::new(&bin)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args([
            "-vn", // no video stream in the output
            "-c:a",
            "libopus",
            "-b:a",
            &bitrate,
            "-application",
            "voip", // speech-optimised; lossy in non-perceptual ways for music
            "-ac",
            "1", // mono — DAISY narration is monaural
            "-ar",
            "16000", // 16 kHz sample rate is plenty for speech
        ])
        .arg(output)
        .status()
        .map_err(Error::Spawn)?;

    if !status.success() {
        return Err(Error::Failed {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            exit_code: status.code(),
        });
    }
    Ok(())
}

/// Convenience: replace the file extension on `path` with `new_ext`
/// (without the leading dot). Used when picking output filenames for the
/// recompressed audio inside a publication.
pub fn with_extension(path: &Path, new_ext: &str) -> PathBuf {
    let mut buf = path.to_path_buf();
    buf.set_extension(new_ext);
    buf
}
