//! On-disk cache for Whisper transcription output.
//!
//! Whisper is the slowest stage of `dpub convert --transcribe`. The
//! output is deterministic given the audio bytes, the model bytes,
//! the language code, and our serialisation schema — so we hash those,
//! key a JSON file by the result, and skip re-running Whisper when
//! the same combination has been seen before.
//!
//! Cache layout:
//! - Directory: `~/.cache/dpub/transcripts/` (Unix), `%LOCALAPPDATA%\dpub\transcripts\` (Windows).
//! - Filename: `<combined_hash>.json` where `combined_hash` derives from
//!   `(audio_sha256, model_sha256, language, schema_version)`.
//! - Format: JSON envelope with diagnostic metadata + the
//!   `Vec<Segment>` payload.
//!
//! The cache is purely an optimisation: read failures fall back to a
//! fresh transcription, write failures are logged and ignored.
//! `DPUB_NO_TRANSCRIPT_CACHE=1` disables both reads and writes.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use dpub_whisper::{Segment, TranscribeOptions, Transcriber};

/// Bumped whenever the on-disk JSON shape changes. Old cache files
/// hash to a different key after a bump and will simply be ignored
/// (and overwritten on the next miss). No deletion needed.
const SCHEMA_VERSION: u32 = 1;

/// Disk cache wrapper around `dpub_whisper::Transcriber`. Keeps the
/// model loaded and its hash memoised across all calls in one run.
pub(crate) struct CachedTranscriber {
    inner: Transcriber,
    model_sha: String,
    language: String,
    cache_dir: PathBuf,
    cache_enabled: bool,
}

impl CachedTranscriber {
    pub(crate) fn new(opts: &TranscribeOptions) -> crate::Result<Self> {
        let inner = Transcriber::new(opts)?;
        let model_sha = hash_file(&opts.model_path).unwrap_or_else(|e| {
            // Hashing failure isn't fatal — it just disables the
            // cache for this run. Log it so the user knows why they
            // didn't get a speedup.
            tracing::warn!(
                "transcript cache: model hash failed ({e}); cache disabled this run"
            );
            String::new()
        });
        let cache_enabled = !model_sha.is_empty()
            && std::env::var_os("DPUB_NO_TRANSCRIPT_CACHE").is_none();
        let cache_dir = transcripts_cache_dir();
        if cache_enabled {
            // Create the dir lazily; ignore failures (we'll log on first write).
            let _ = fs::create_dir_all(&cache_dir);
        }
        Ok(Self {
            inner,
            model_sha,
            language: opts.language.clone(),
            cache_dir,
            cache_enabled,
        })
    }

    pub(crate) fn transcribe(&self, audio_path: &Path) -> crate::Result<Vec<Segment>> {
        if !self.cache_enabled {
            return Ok(self.inner.transcribe(audio_path)?);
        }
        let audio_sha = match hash_file(audio_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "transcript cache: audio hash failed for {} ({e}); transcribing without cache",
                    audio_path.display()
                );
                return Ok(self.inner.transcribe(audio_path)?);
            }
        };
        let key = combined_key(&audio_sha, &self.model_sha, &self.language);
        let cache_path = self.cache_dir.join(format!("{key}.json"));

        if let Some(segments) = read_cached(&cache_path) {
            tracing::info!(
                "transcript cache: hit for {} ({} segments)",
                audio_path.display(),
                segments.len()
            );
            return Ok(segments);
        }

        let segments = self.inner.transcribe(audio_path)?;
        let envelope = Envelope {
            schema_version: SCHEMA_VERSION,
            audio_sha256: audio_sha,
            model_sha256: self.model_sha.clone(),
            language: self.language.clone(),
            dpub_whisper_version: env!("CARGO_PKG_VERSION").to_owned(),
            segments: segments.clone(),
        };
        if let Err(e) = write_cached(&cache_path, &envelope) {
            tracing::warn!(
                "transcript cache: write failed for {} ({e}); transcript will be re-computed next time",
                cache_path.display()
            );
        } else {
            tracing::debug!(
                "transcript cache: stored {} ({} segments)",
                cache_path.display(),
                envelope.segments.len()
            );
        }
        Ok(segments)
    }
}

/// JSON envelope written to disk. The metadata fields duplicate the
/// inputs that already feed into the cache key — they're for `jq`
/// debugging, not lookup.
#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    schema_version: u32,
    audio_sha256: String,
    model_sha256: String,
    language: String,
    dpub_whisper_version: String,
    segments: Vec<Segment>,
}

/// Look up the cache file. Returns `Some(segments)` on a clean hit.
/// Any error (missing file, corrupt JSON, schema mismatch) yields
/// `None`; missing files are silent, real errors log a warning.
fn read_cached(path: &Path) -> Option<Vec<Segment>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("transcript cache: read failed for {}: {e}", path.display());
            return None;
        }
    };
    let env: Envelope = match serde_json::from_slice(&bytes) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                "transcript cache: ignoring malformed entry {}: {e}",
                path.display()
            );
            return None;
        }
    };
    if env.schema_version != SCHEMA_VERSION {
        return None;
    }
    Some(env.segments)
}

/// Atomically write the cache entry (`.partial` then rename). Same
/// pattern as the model downloader in `dpub-cli/src/setup.rs`.
fn write_cached(path: &Path, envelope: &Envelope) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("json.partial");
    let json = serde_json::to_vec(envelope).map_err(std::io::Error::other)?;
    {
        let mut f = fs::File::create(&partial)?;
        f.write_all(&json)?;
        f.sync_data()?;
    }
    fs::rename(&partial, path)?;
    Ok(())
}

/// Stream-hash a file's bytes with SHA-256. Mirrors the helper used
/// for `dpub setup --whisper-model …` model verification but lives
/// here to avoid a cross-crate dependency for ~15 lines.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(hasher.finalize().as_slice()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Combined cache key: `sha256(audio_sha || model_sha || lang || schema_version)`,
/// truncated to 32 hex chars. Truncation is fine: SHA-256 has no
/// adversary here, only the normal birthday-bound risk, which at 128
/// bits of entropy is ~2^64 inputs before a collision is even
/// plausible. Real-world cache will have a few thousand entries max.
fn combined_key(audio_sha: &str, model_sha: &str, language: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(audio_sha.as_bytes());
    hasher.update(b"\0");
    hasher.update(model_sha.as_bytes());
    hasher.update(b"\0");
    hasher.update(language.as_bytes());
    hasher.update(b"\0");
    hasher.update(SCHEMA_VERSION.to_le_bytes());
    let hex = hex(hasher.finalize().as_slice());
    hex[..32].to_owned()
}

/// Return the platform-appropriate transcripts cache directory.
/// Mirrors the layout of `~/.cache/dpub/models/` in `dpub-cli/setup.rs`.
fn transcripts_cache_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        let base = std::env::var_os("LOCALAPPDATA")
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        base.join("dpub").join("transcripts")
    } else {
        let home = std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        home.join(".cache").join("dpub").join("transcripts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpub_whisper::Word;

    fn sample_segments() -> Vec<Segment> {
        vec![Segment {
            start_seconds: 0.0,
            end_seconds: 1.5,
            text: "Hello world.".into(),
            words: vec![
                Word {
                    start_seconds: 0.0,
                    end_seconds: 0.5,
                    text: "Hello".into(),
                },
                Word {
                    start_seconds: 0.5,
                    end_seconds: 1.5,
                    text: "world.".into(),
                },
            ],
        }]
    }

    #[test]
    fn round_trip_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entry.json");
        let env = Envelope {
            schema_version: SCHEMA_VERSION,
            audio_sha256: "aaaa".into(),
            model_sha256: "bbbb".into(),
            language: "nl".into(),
            dpub_whisper_version: "0.6.0".into(),
            segments: sample_segments(),
        };
        write_cached(&path, &env).unwrap();
        let got = read_cached(&path).expect("hit");
        assert_eq!(got, env.segments);
    }

    #[test]
    fn missing_file_is_silent_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert!(read_cached(&path).is_none());
    }

    #[test]
    fn corrupt_file_is_warning_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, b"not json").unwrap();
        assert!(read_cached(&path).is_none());
    }

    #[test]
    fn schema_mismatch_treated_as_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v0.json");
        let json = serde_json::json!({
            "schema_version": SCHEMA_VERSION + 99,
            "audio_sha256": "a",
            "model_sha256": "b",
            "language": "nl",
            "dpub_whisper_version": "0.6.0",
            "segments": [],
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(read_cached(&path).is_none());
    }

    #[test]
    fn hash_file_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin");
        fs::write(&p, b"hello world").unwrap();
        assert_eq!(hash_file(&p).unwrap(), hash_file(&p).unwrap());
    }

    #[test]
    fn hash_file_distinguishes_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        fs::write(&a, b"hello").unwrap();
        fs::write(&b, b"world").unwrap();
        assert_ne!(hash_file(&a).unwrap(), hash_file(&b).unwrap());
    }

    #[test]
    fn combined_key_changes_when_any_input_changes() {
        let base = combined_key("aaaa", "bbbb", "nl");
        assert_ne!(base, combined_key("zzzz", "bbbb", "nl"));
        assert_ne!(base, combined_key("aaaa", "zzzz", "nl"));
        assert_ne!(base, combined_key("aaaa", "bbbb", "en"));
    }

    #[test]
    fn cache_dir_ends_in_transcripts() {
        let dir = transcripts_cache_dir();
        assert_eq!(dir.file_name().unwrap(), "transcripts");
        let parent_name = dir.parent().unwrap().file_name().unwrap();
        assert_eq!(parent_name, "dpub");
    }
}
