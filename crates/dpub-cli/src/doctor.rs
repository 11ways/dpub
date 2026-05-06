//! `dpub doctor` — diagnostic for build state, runtime tools, and
//! cached Whisper models.
//!
//! Read-only by default; with `--install` (handled in `main.rs`) it
//! offers to invoke the platform's package manager to fill missing
//! tools.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::setup;

/// One row in the diagnostic report.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub key: &'static str,
    pub label: &'static str,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warning,
    Missing,
}

impl Status {
    pub fn glyph(self) -> &'static str {
        match self {
            Status::Ok => "✓",
            Status::Warning => "⚠",
            Status::Missing => "✗",
        }
    }
}

/// Aggregated report. Tools are emitted in display order.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub dpub_version: &'static str,
    pub gpu_acceleration: &'static str,
    pub tools: Vec<Tool>,
}

/// Run every detector and return the aggregated report.
pub fn diagnose() -> Report {
    Report {
        dpub_version: env!("CARGO_PKG_VERSION"),
        gpu_acceleration: gpu_label(),
        tools: vec![
            check_convert(),
            check_epubcheck(),
            check_ace(),
            check_ffmpeg(),
            check_whisper_model(),
        ],
    }
}

fn gpu_label() -> &'static str {
    if cfg!(feature = "metal") {
        "Metal"
    } else if cfg!(feature = "cuda") {
        "CUDA"
    } else {
        "CPU only"
    }
}

fn check_convert() -> Tool {
    // Conversion has no required external tool — the binary itself is
    // sufficient. We surface the row anyway so the report has a
    // visible "core conversion is fine" line.
    Tool {
        key: "convert",
        label: "DAISY → EPUB conversion",
        status: Status::Ok,
        version: None,
        detail: Some("ready (built-in, no external dependency)".into()),
        install_hint: None,
    }
}

fn check_epubcheck() -> Tool {
    let label = "EPUB validation (epubcheck)";
    let Ok(path) = which::which("epubcheck") else {
        return Tool {
            key: "epubcheck",
            label,
            status: Status::Missing,
            version: None,
            detail: Some("not on PATH".into()),
            install_hint: Some(install_hint_for(&EpubcheckHint).into()),
        };
    };
    let version = run_version(&path, &["--version"]).map(|s| {
        // `EPUBCheck v5.3.0` → `5.3.0`
        s.trim_start_matches("EPUBCheck v")
            .trim_start_matches("EPUBCheck ")
            .to_owned()
    });
    Tool {
        key: "epubcheck",
        label,
        status: Status::Ok,
        version,
        detail: None,
        install_hint: None,
    }
}

fn check_ace() -> Tool {
    let label = "Accessibility (ace)";
    let Ok(path) = which::which("ace") else {
        return Tool {
            key: "ace",
            label,
            status: Status::Missing,
            version: None,
            detail: Some("not on PATH".into()),
            install_hint: Some(install_hint_for(&AceHint).into()),
        };
    };
    let version = run_version(&path, &["--version"]);
    Tool {
        key: "ace",
        label,
        status: Status::Ok,
        version,
        detail: None,
        install_hint: None,
    }
}

fn check_ffmpeg() -> Tool {
    let label = "Audio recompression (ffmpeg)";
    let Ok(path) = which::which("ffmpeg") else {
        return Tool {
            key: "ffmpeg",
            label,
            status: Status::Missing,
            version: None,
            detail: Some("not on PATH".into()),
            install_hint: Some(install_hint_for(&FfmpegHint).into()),
        };
    };
    let version = run_version(&path, &["-version"]).and_then(|s| {
        // `ffmpeg version 8.1.1 Copyright (c) ...` → `8.1.1`
        let rest = s.trim_start_matches("ffmpeg version ").trim();
        rest.split_whitespace().next().map(str::to_owned)
    });
    Tool {
        key: "ffmpeg",
        label,
        status: Status::Ok,
        version,
        detail: None,
        install_hint: None,
    }
}

fn check_whisper_model() -> Tool {
    let label = "Whisper transcription";
    match setup::list_cached_models() {
        Ok(models) if !models.is_empty() => {
            let names: Vec<String> = models
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            Tool {
                key: "whisper-model",
                label,
                status: Status::Ok,
                version: None,
                detail: Some(format!("{} cached: {}", models.len(), names.join(", "))),
                install_hint: None,
            }
        }
        Ok(_) => Tool {
            key: "whisper-model",
            label,
            status: Status::Warning,
            version: None,
            detail: Some(format!("no model in {}", setup::cache_dir().display())),
            install_hint: Some("dpub setup --whisper-model medium".into()),
        },
        Err(e) => Tool {
            key: "whisper-model",
            label,
            status: Status::Warning,
            version: None,
            detail: Some(format!("cache check failed: {e}")),
            install_hint: Some("dpub setup --whisper-model medium".into()),
        },
    }
}

/// Run `<bin> <args...>` and return its first line of stdout, trimmed.
/// Used for `--version` parsing. Returns `None` on any failure.
fn run_version(bin: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(bin).args(args).output().ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    let first_non_empty = s.lines().find(|l| !l.trim().is_empty())?;
    Some(first_non_empty.trim().to_owned())
}

/// Marker trait for per-tool install hints. Each variant produces a
/// platform-appropriate one-line install command for the host.
trait InstallHint {
    fn for_host(&self) -> &'static str;
}

struct EpubcheckHint;
struct AceHint;
struct FfmpegHint;

impl InstallHint for EpubcheckHint {
    fn for_host(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "brew install epubcheck"
        } else if cfg!(target_os = "linux") {
            "see https://github.com/w3c/epubcheck/releases (also needs Java 11)"
        } else {
            "https://github.com/w3c/epubcheck/releases (also needs Java 11)"
        }
    }
}

impl InstallHint for AceHint {
    fn for_host(&self) -> &'static str {
        // Same command on all platforms; npm hides the difference.
        "npm install -g @daisy/ace"
    }
}

impl InstallHint for FfmpegHint {
    fn for_host(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "brew install ffmpeg"
        } else if cfg!(target_os = "linux") {
            "sudo apt-get install -y ffmpeg  # or: sudo dnf install -y ffmpeg"
        } else {
            "https://ffmpeg.org/download.html"
        }
    }
}

fn install_hint_for(h: &impl InstallHint) -> &'static str {
    h.for_host()
}

/// Render the human-readable doctor output to stdout. Mirrors
/// `print_report` in `dpub_validate` for visual consistency.
pub fn print_report(report: &Report) {
    println!(
        "Build:                       ✓ dpub {} (GPU: {})",
        report.dpub_version, report.gpu_acceleration,
    );
    for tool in &report.tools {
        let glyph = tool.status.glyph();
        let label = tool.label;
        // Pad to 28 character columns so the glyph aligns vertically.
        // Use char count (not byte length) so multi-byte chars like the
        // `→` in "DAISY → EPUB conversion" are counted as one column.
        let mut line = format!("{label}:");
        while line.chars().count() < 29 {
            line.push(' ');
        }
        let mut detail_parts = Vec::new();
        if let Some(v) = &tool.version {
            detail_parts.push(v.clone());
        }
        if let Some(d) = &tool.detail {
            detail_parts.push(d.clone());
        }
        let detail = if detail_parts.is_empty() {
            "ready".to_owned()
        } else {
            detail_parts.join(" — ")
        };
        println!("{line}{glyph} {detail}");
        if let Some(hint) = &tool.install_hint {
            println!("                                install: {hint}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_label_resolves_to_a_known_string() {
        let label = gpu_label();
        assert!(matches!(label, "Metal" | "CUDA" | "CPU only"));
    }

    #[test]
    fn report_serializes_to_json() {
        let report = diagnose();
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(json.contains("\"dpub_version\""));
        assert!(json.contains("\"gpu_acceleration\""));
        assert!(json.contains("\"tools\""));
    }

    #[test]
    fn convert_row_is_always_ok() {
        let row = check_convert();
        assert_eq!(row.status, Status::Ok);
        assert_eq!(row.key, "convert");
    }
}
