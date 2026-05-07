//! `dpub setup --whisper-model <size>` — download a GGML Whisper
//! model into a per-user cache directory, with SHA256 verification.
//!
//! Cache layout:
//! - macOS / Linux: `$HOME/.cache/dpub/models/ggml-<size>.bin`
//! - Windows: `%LOCALAPPDATA%\dpub\models\ggml-<size>.bin`
//!
//! `dpub convert --transcribe <lang>` (without `--whisper-model`)
//! auto-discovers the most recently modified model in the cache dir.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Known whisper.cpp GGML model sizes that this command can download.
/// SHA256s come from the upstream Hugging Face mirror; bumped when
/// upstream rotates a model.
pub const KNOWN_MODELS: &[ModelSpec] = &[
    ModelSpec {
        size: "tiny",
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        bytes: 77_691_713,
    },
    ModelSpec {
        size: "base",
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        bytes: 147_951_465,
    },
    ModelSpec {
        size: "small",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        bytes: 487_601_967,
    },
    ModelSpec {
        size: "medium",
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        bytes: 1_533_763_059,
    },
    ModelSpec {
        size: "large-v3",
        sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        bytes: 3_094_623_691,
    },
];

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub size: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

impl ModelSpec {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
            self.size
        )
    }

    pub fn filename(&self) -> String {
        format!("ggml-{}.bin", self.size)
    }
}

pub fn lookup(size: &str) -> Option<&'static ModelSpec> {
    KNOWN_MODELS.iter().find(|m| m.size == size)
}

pub fn known_size_names() -> Vec<&'static str> {
    KNOWN_MODELS.iter().map(|m| m.size).collect()
}

/// Resolve dpub's per-user model cache directory. Creates it lazily;
/// callers should still expect `std::io::Error` on disk-full etc.
pub fn cache_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        let base = std::env::var_os("LOCALAPPDATA").map_or_else(
            || PathBuf::from("."),
            PathBuf::from,
        );
        base.join("dpub").join("models")
    } else {
        let home = std::env::var_os("HOME").map_or_else(
            || PathBuf::from("."),
            PathBuf::from,
        );
        home.join(".cache").join("dpub").join("models")
    }
}

/// Return paths of every `ggml-*.bin` file in the cache dir, sorted
/// by most-recently-modified first. Returns an empty `Vec` if the
/// cache dir doesn't exist.
pub fn list_cached_models() -> std::io::Result<Vec<PathBuf>> {
    let dir = cache_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ggml-") || !name.to_ascii_lowercase().ends_with(".bin") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        out.push((mtime, path));
    }
    out.sort_by_key(|(t, _)| std::cmp::Reverse(*t)); // newest first
    Ok(out.into_iter().map(|(_, p)| p).collect())
}

/// Most-recently-modified model in the cache, if any.
pub fn most_recent_model() -> Option<PathBuf> {
    list_cached_models().ok().and_then(|v| v.into_iter().next())
}

/// Download `spec` into the cache dir, verifying SHA256. Atomic via
/// a `<dest>.partial` rename; on hash mismatch the partial file is
/// deleted and an error is returned. Skips the download if the dest
/// already exists with a matching hash.
pub fn install_model(spec: &ModelSpec) -> Result<PathBuf> {
    let dir = cache_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("creating cache dir {}", dir.display()))?;
    let final_path = dir.join(spec.filename());
    let partial_path = dir.join(format!("{}.partial", spec.filename()));

    if final_path.is_file() {
        eprintln!(
            "Verifying existing {} ...",
            final_path.file_name().unwrap_or_default().to_string_lossy()
        );
        match verify_sha256(&final_path, spec.sha256) {
            Ok(true) => {
                eprintln!("Already cached and verified: {}", final_path.display());
                return Ok(final_path);
            }
            Ok(false) => {
                eprintln!("Existing file failed SHA check; re-downloading.");
                fs::remove_file(&final_path).ok();
            }
            Err(e) => {
                eprintln!("Verifying existing file failed ({e}); re-downloading.");
                fs::remove_file(&final_path).ok();
            }
        }
    }

    eprintln!(
        "Downloading {} ({}) ...",
        spec.filename(),
        format_bytes(spec.bytes),
    );
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        if attempt > 1 {
            eprintln!("  Retrying (attempt {attempt}/{MAX_ATTEMPTS}) ...");
            std::thread::sleep(Duration::from_secs(2));
        }
        match download_once(spec, &partial_path) {
            Ok(hasher) => {
                last_err = None;
                // Verify SHA256 before promoting to final path.
                let actual = hex(hasher.finalize().as_slice());
                if actual != spec.sha256 {
                    fs::remove_file(&partial_path).ok();
                    anyhow::bail!(
                        "SHA256 mismatch for {}: expected {}, got {}",
                        spec.filename(),
                        spec.sha256,
                        actual,
                    );
                }
                break;
            }
            Err(e) => {
                eprintln!(); // newline after stalled progress bar
                eprintln!("  Download interrupted: {e:#}");
                fs::remove_file(&partial_path).ok();
                last_err = Some(e);
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e).with_context(|| {
            format!(
                "downloading {} failed after {MAX_ATTEMPTS} attempts",
                spec.url()
            )
        });
    }

    fs::rename(&partial_path, &final_path)
        .with_context(|| format!("renaming {} → {}", partial_path.display(), final_path.display()))?;
    eprintln!("Verified SHA256.");
    eprintln!("Cached: {}", final_path.display());
    Ok(final_path)
}

/// Single download attempt. Returns the SHA256 hasher on success so the
/// caller can verify the hash. On I/O or network error the partial file
/// is left on disk (the caller decides whether to retry or clean up).
fn download_once(
    spec: &ModelSpec,
    partial_path: &Path,
) -> Result<Sha256> {
    let agent = dpub_meta::agent();
    let mut hasher = Sha256::new();
    let mut file = fs::File::create(partial_path)
        .with_context(|| format!("creating {}", partial_path.display()))?;
    let mut last_tick = Instant::now();
    let started = Instant::now();
    {
        let mut tee = HashingWriter {
            inner: &mut file,
            hasher: &mut hasher,
        };
        dpub_meta::download_to_writer(&agent, &spec.url(), &mut tee, |bytes, total| {
            let now = Instant::now();
            if now.duration_since(last_tick) < Duration::from_millis(250)
                && bytes < total.max(spec.bytes)
            {
                return;
            }
            last_tick = now;
            render_progress(bytes, total.max(spec.bytes), started);
        })
        .with_context(|| format!("downloading {}", spec.url()))?;
    }
    eprintln!(); // newline after the in-place progress bar
    file.flush().ok();
    drop(file);
    Ok(hasher)
}

/// Verify an existing file's SHA256 against `expected_hex` without
/// re-downloading.
fn verify_sha256(path: &Path, expected_hex: &str) -> Result<bool> {
    use std::io::Read;
    let mut file = fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(hasher.finalize().as_slice()) == expected_hex)
}

struct HashingWriter<'a> {
    inner: &'a mut fs::File,
    hasher: &'a mut Sha256,
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn render_progress(bytes: u64, total: u64, started: Instant) {
    let total = total.max(1);
    #[allow(clippy::cast_precision_loss)]
    let pct = ((bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
    let bar_width: usize = 40;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let filled = filled.min(bar_width);
    let bar: String = std::iter::repeat_n('#', filled)
        .chain(std::iter::repeat_n('-', bar_width - filled))
        .collect();
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    #[allow(clippy::cast_precision_loss)]
    let mbps = (bytes as f64 / 1_048_576.0) / elapsed;
    eprint!(
        "\r  [{bar}] {pct:>5.1}% ({} of {}, {mbps:.1} MiB/s)        ",
        format_bytes(bytes),
        format_bytes(total),
    );
}

fn format_bytes(n: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let f = n as f64;
    if n >= 1_000_000_000 {
        format!("{:.2} GB", f / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1} MB", f / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0} KB", f / 1_000.0)
    } else {
        format!("{n} B")
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_finds_known_sizes() {
        assert!(lookup("medium").is_some());
        assert!(lookup("tiny").is_some());
        assert!(lookup("nonsense").is_none());
    }

    #[test]
    fn cache_dir_is_under_home_or_localappdata() {
        let dir = cache_dir();
        assert!(dir.ends_with("dpub/models") || dir.ends_with("dpub\\models"));
    }

    #[test]
    fn known_size_names_includes_medium() {
        let names = known_size_names();
        assert!(names.contains(&"medium"));
    }

    #[test]
    fn format_bytes_human_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_500), "2 KB");
        assert!(format_bytes(1_500_000).starts_with("1.5"));
        assert!(format_bytes(1_500_000_000).contains("GB"));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex(&[]), "");
    }
}
