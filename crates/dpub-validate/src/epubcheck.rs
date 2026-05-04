//! EPUBCheck integration.
//!
//! `epubcheck --json - <publication>` writes a structured report to stdout.
//! We parse it into our own types so the caller doesn't have to depend on
//! EPUBCheck's wire format.
//!
//! Anything we don't recognise is silently dropped — the goal is a stable
//! summary, not a 1:1 mirror of EPUBCheck's output. Callers who need the
//! raw report can run `epubcheck` themselves.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::report::{BackendReport, Issue, Severity, Summary};

/// `true` if `epubcheck` is available on `PATH`.
pub fn epubcheck_available() -> bool {
    which::which("epubcheck").is_ok()
}

/// Run EPUBCheck against `epub_path` and return the parsed report.
pub fn run_epubcheck(epub_path: &Path) -> Result<BackendReport> {
    let bin = which::which("epubcheck").map_err(|_| Error::EpubcheckMissing)?;

    let output = Command::new(&bin)
        .arg(epub_path)
        .arg("--json")
        .arg("-")
        .output()
        .map_err(Error::Spawn)?;

    // EPUBCheck exits non-zero when it found errors. That isn't a failure of
    // *us* — it's literally what we asked for. So we accept any exit code as
    // long as we got some JSON on stdout. The only exit we treat as a hard
    // failure is "no JSON at all" (e.g. binary couldn't open the file).
    if output.stdout.is_empty() {
        return Err(Error::Exited {
            path: epub_path.to_path_buf(),
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let raw: RawReport = serde_json::from_slice(&output.stdout)?;
    Ok(into_report(raw))
}

fn into_report(raw: RawReport) -> BackendReport {
    let summary = Summary {
        fatals: raw.checker.n_fatal.unwrap_or(0),
        errors: raw.checker.n_error.unwrap_or(0),
        warnings: raw.checker.n_warning.unwrap_or(0),
        infos: raw.checker.n_usage.unwrap_or(0),
    };

    let issues = raw
        .messages
        .into_iter()
        .map(|m| {
            let severity = parse_severity(&m.severity);
            let location = m.locations.first().and_then(|l| {
                let path = l.path.as_deref()?;
                let line = l.line.unwrap_or(0);
                let col = l.column.unwrap_or(0);
                Some(if line > 0 {
                    format!("{path}:{line}:{col}")
                } else {
                    path.to_owned()
                })
            });
            Issue {
                severity,
                id: m.id,
                message: m.message.unwrap_or_default(),
                location,
            }
        })
        .collect();

    BackendReport {
        backend: "epubcheck".into(),
        version: raw.checker.checker_version,
        summary,
        issues,
    }
}

fn parse_severity(s: &str) -> Severity {
    // Unknown severities fall through to `Error` so new EPUBCheck variants
    // never silently disappear.
    match s.to_ascii_uppercase().as_str() {
        "FATAL" => Severity::Fatal,
        "WARNING" => Severity::Warning,
        "INFO" => Severity::Info,
        "USAGE" => Severity::Usage,
        "SUPPRESSED" => Severity::Suppressed,
        _ => Severity::Error,
    }
}

// ----------------- raw EPUBCheck JSON shapes -----------------
//
// Defensive deserialisation: every field is optional so future EPUBCheck
// schema changes degrade to "missing data" instead of total failure.

#[derive(Deserialize)]
struct RawReport {
    #[serde(default)]
    checker: RawChecker,
    #[serde(default)]
    messages: Vec<RawMessage>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawChecker {
    checker_version: Option<String>,
    n_fatal: Option<u32>,
    n_error: Option<u32>,
    n_warning: Option<u32>,
    n_usage: Option<u32>,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(rename = "ID")]
    id: Option<String>,
    severity: String,
    message: Option<String>,
    #[serde(default)]
    locations: Vec<RawLocation>,
}

#[derive(Deserialize)]
struct RawLocation {
    path: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}
