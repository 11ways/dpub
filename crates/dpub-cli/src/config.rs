use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Transcription setting in the config file. Accepts:
/// - `"nl"` / `"en"` / ... → always transcribe with this language
/// - `true` → transcribe, auto-detect language from book metadata
/// - `false` or `null` → do not transcribe
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TranscribeSetting {
    /// Auto-detect language from book metadata when `true`.
    Auto(bool),
    /// Explicit language code.
    Language(String),
}

/// Persistent user defaults for dpub. Every field is optional — a missing
/// key in the JSON simply falls through to the hard-coded default.
///
/// The canonical location is `~/.config/dpub/config.json` on Unix and
/// `%APPDATA%\dpub\config.json` on Windows. CLI flags always override.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DpubConfig {
    /// Audio handling: `"original"` or `"opus"`.
    pub audio: Option<String>,
    /// Opus bitrate in kbit/s (32–96 for speech).
    pub bitrate: Option<u32>,
    /// Enable automatic cover lookup via Open Library.
    pub auto_cover: Option<bool>,
    /// Skip per-word Media Overlay sync (fall back to per-paragraph).
    pub no_word_sync: Option<bool>,
    /// Default rights statement for `<dc:rights>`.
    pub rights: Option<String>,
    /// Path to a `ggml-*.bin` Whisper model file.
    pub whisper_model: Option<PathBuf>,
    /// Transcription: `true` (auto-detect language), `"nl"` (explicit), or `null` (off).
    pub transcribe: Option<TranscribeSetting>,
    /// Run EPUBCheck after conversion.
    pub validate: Option<bool>,
    /// Run DAISY ACE after conversion.
    pub a11y: Option<bool>,
    /// Parallel batch job count (`0` = let rayon decide).
    pub jobs: Option<usize>,
    /// Default log level (`"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"`).
    pub log_level: Option<String>,
}

/// Return the platform-appropriate config directory for dpub.
///
/// - Unix: `$HOME/.config/dpub/`
/// - Windows: `%APPDATA%\dpub\`
pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        let base = std::env::var_os("APPDATA")
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        base.join("dpub")
    } else {
        let home = std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        home.join(".config").join("dpub")
    }
}

/// Full path to the config file.
pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Load the config file. Returns `Default` (all `None`) when the file is
/// absent or unparseable — dpub should never fail to start because of a
/// broken config.
pub fn load() -> DpubConfig {
    load_from(&config_path())
}

fn load_from(path: &Path) -> DpubConfig {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DpubConfig::default(),
        Err(e) => {
            tracing::warn!("could not read {}: {e}", path.display());
            return DpubConfig::default();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("ignoring {}: {e}", path.display());
            DpubConfig::default()
        }
    }
}

/// Example JSON for `dpub config` output and `--init`.
pub fn example_json() -> &'static str {
    r#"{
  "audio": "original",
  "bitrate": 64,
  "auto_cover": true,
  "no_word_sync": false,
  "rights": null,
  "whisper_model": null,
  "transcribe": null,
  "validate": false,
  "a11y": false,
  "jobs": 0,
  "log_level": "info"
}"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_default() {
        let cfg = load_from(std::path::Path::new("/tmp/dpub-test-nonexistent/config.json"));
        assert!(cfg.audio.is_none());
        assert!(cfg.bitrate.is_none());
        assert!(cfg.auto_cover.is_none());
    }

    #[test]
    fn partial_json_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"bitrate": 48}"#).unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg.bitrate, Some(48));
        assert!(cfg.audio.is_none());
        assert!(cfg.auto_cover.is_none());
    }

    #[test]
    fn invalid_json_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json at all").unwrap();
        let cfg = load_from(&path);
        assert!(cfg.audio.is_none());
    }

    #[test]
    fn full_json_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, example_json()).unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg.audio.as_deref(), Some("original"));
        assert_eq!(cfg.bitrate, Some(64));
        assert_eq!(cfg.auto_cover, Some(true));
        assert_eq!(cfg.validate, Some(false));
        assert_eq!(cfg.log_level.as_deref(), Some("info"));
        assert!(cfg.transcribe.is_none());
    }

    #[test]
    fn transcribe_accepts_bool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"transcribe": true}"#).unwrap();
        let cfg = load_from(&path);
        assert!(matches!(cfg.transcribe, Some(TranscribeSetting::Auto(true))));
    }

    #[test]
    fn transcribe_accepts_string() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"transcribe": "nl"}"#).unwrap();
        let cfg = load_from(&path);
        assert!(matches!(cfg.transcribe, Some(TranscribeSetting::Language(ref s)) if s == "nl"));
    }

    #[test]
    fn config_dir_ends_with_dpub() {
        let dir = config_dir();
        assert_eq!(dir.file_name().unwrap(), "dpub");
    }
}
