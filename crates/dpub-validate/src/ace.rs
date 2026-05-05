//! ACE (Accessibility Checker for EPUB) integration.
//!
//! `ace` is a Node.js tool from the DAISY Consortium. It runs the
//! [axe-core](https://github.com/dequesystems/axe-core) WCAG engine on
//! every content document in the publication and adds EPUB-specific
//! checks (presence of accessibility metadata, alt text on cover, page
//! break anchors, …). EPUBCheck validates the format; ACE validates the
//! accessibility. Both matter under the European Accessibility Act and
//! comparable regimes (Section 508, AODA).
//!
//! Install with `npm install -g @daisy/ace`. The wrapper is opt-in
//! (only fires when the binary is on `PATH`).
//!
//! Output: ACE writes `<outdir>/report.json` in EARL/JSON-LD form. We
//! parse the bits we care about — pass/fail outcomes per assertion —
//! and silently drop everything else, so future ACE schema additions
//! degrade to "missing data" rather than break the wrapper.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::report::{BackendReport, Issue, Severity, Summary};

/// `true` if `ace` is on `PATH`. Install with `npm install -g @daisy/ace`.
pub fn ace_available() -> bool {
    which::which("ace").is_ok()
}

/// Run ACE against `epub_path` and return the parsed report.
///
/// ACE writes its output to a directory; we use a tempdir, read
/// `report.json`, and discard the rest.
pub fn run_ace(epub_path: &Path) -> Result<BackendReport> {
    let bin = which::which("ace").map_err(|_| Error::AceMissing)?;
    let outdir = tempfile::tempdir()?;

    let output = Command::new(&bin)
        .arg("-o")
        .arg(outdir.path())
        .arg("-s") // silent — don't print to stdout
        .arg("-f") // force — overwrite outdir if it exists
        .arg(epub_path)
        .output()
        .map_err(Error::Spawn)?;

    let report_path = outdir.path().join("report.json");
    if !report_path.is_file() {
        return Err(Error::Exited {
            path: epub_path.to_path_buf(),
            message: format!(
                "ace did not produce report.json (exit {:?}): {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr),
            ),
        });
    }

    let body = std::fs::read(&report_path)?;
    let raw: RawReport = serde_json::from_slice(&body)?;
    Ok(into_report(raw))
}

fn into_report(raw: RawReport) -> BackendReport {
    let mut issues = Vec::new();
    let mut errors = 0u32;
    let mut warnings = 0u32;

    for spine_assertion in &raw.assertions {
        let location = spine_assertion
            .test_subject
            .as_ref()
            .and_then(|s| s.url.clone());
        for inner in &spine_assertion.assertions {
            let outcome = inner
                .result
                .as_ref()
                .and_then(|r| r.outcome.as_deref())
                .unwrap_or("");
            let severity = match outcome {
                "earl:failed" => Severity::Error,
                "earl:cantTell" => Severity::Warning,
                _ => continue, // earl:passed / earl:inapplicable / unknown — skip
            };
            match severity {
                Severity::Error => errors += 1,
                Severity::Warning => warnings += 1,
                _ => {}
            }
            let id = inner
                .test
                .as_ref()
                .and_then(|t| t.title.clone());
            let message = inner
                .test
                .as_ref()
                .and_then(|t| t.description.clone())
                .or_else(|| inner.result.as_ref().and_then(|r| r.description.clone()))
                .unwrap_or_default();
            issues.push(Issue {
                severity,
                id,
                message,
                location: location.clone(),
            });
        }
    }

    BackendReport {
        backend: "ace".into(),
        version: raw.asserted_by.and_then(RawAsserter::version),
        summary: Summary {
            fatals: 0,
            errors,
            warnings,
            infos: 0,
        },
        issues,
    }
}

// ----------------- raw ACE report shapes -----------------
//
// EARL/JSON-LD; we keep only the fields we actually use and let
// everything else degrade to defaults.

#[derive(Deserialize, Default)]
struct RawReport {
    #[serde(default, rename = "earl:assertedBy")]
    asserted_by: Option<RawAsserter>,
    #[serde(default)]
    assertions: Vec<RawSpineAssertion>,
}

#[derive(Deserialize, Default)]
struct RawAsserter {
    #[serde(default, rename = "doap:release")]
    release: Option<RawRelease>,
    #[serde(default, rename = "doap:revision")]
    revision: Option<String>,
}

impl RawAsserter {
    fn version(self) -> Option<String> {
        self.release
            .and_then(|r| r.revision)
            .or(self.revision)
    }
}

#[derive(Deserialize, Default)]
struct RawRelease {
    #[serde(default, rename = "doap:revision")]
    revision: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawSpineAssertion {
    #[serde(default, rename = "earl:testSubject")]
    test_subject: Option<RawTestSubject>,
    #[serde(default)]
    assertions: Vec<RawInnerAssertion>,
}

#[derive(Deserialize, Default)]
struct RawTestSubject {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawInnerAssertion {
    #[serde(default, rename = "earl:test")]
    test: Option<RawTest>,
    #[serde(default, rename = "earl:result")]
    result: Option<RawResult>,
}

#[derive(Deserialize, Default)]
struct RawTest {
    #[serde(default, rename = "dct:title")]
    title: Option<String>,
    #[serde(default, rename = "dct:description")]
    description: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawResult {
    #[serde(default, rename = "earl:outcome")]
    outcome: Option<String>,
    #[serde(default, rename = "dct:description")]
    description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_failed_assertion() {
        // Hand-crafted shape mirroring real ACE output. Two leaf
        // assertions: one failed (image-alt) and one passed (label).
        let body = serde_json::json!({
            "earl:assertedBy": {
                "doap:release": {"doap:revision": "1.3.4"}
            },
            "assertions": [
                {
                    "earl:testSubject": {"url": "EPUB/section-001.xhtml"},
                    "assertions": [
                        {
                            "earl:test": {
                                "dct:title": "image-alt",
                                "dct:description": "Ensures <img> elements have alternate text"
                            },
                            "earl:result": {"earl:outcome": "earl:failed"}
                        },
                        {
                            "earl:test": {"dct:title": "label"},
                            "earl:result": {"earl:outcome": "earl:passed"}
                        }
                    ]
                }
            ]
        })
        .to_string();
        let raw: RawReport = serde_json::from_str(&body).expect("parse");
        let report = into_report(raw);

        assert_eq!(report.backend, "ace");
        assert_eq!(report.version.as_deref(), Some("1.3.4"));
        assert_eq!(report.summary.errors, 1);
        assert_eq!(report.summary.warnings, 0);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].id.as_deref(), Some("image-alt"));
        assert_eq!(
            report.issues[0].location.as_deref(),
            Some("EPUB/section-001.xhtml")
        );
    }

    #[test]
    fn cant_tell_outcome_becomes_warning() {
        let body = serde_json::json!({
            "assertions": [{
                "earl:testSubject": {"url": "EPUB/x.xhtml"},
                "assertions": [{
                    "earl:test": {"dct:title": "color-contrast"},
                    "earl:result": {"earl:outcome": "earl:cantTell"}
                }]
            }]
        })
        .to_string();
        let raw: RawReport = serde_json::from_str(&body).expect("parse");
        let report = into_report(raw);
        assert_eq!(report.summary.errors, 0);
        assert_eq!(report.summary.warnings, 1);
        assert_eq!(report.issues[0].severity, Severity::Warning);
    }

    #[test]
    fn passed_and_inapplicable_outcomes_are_silent() {
        let body = serde_json::json!({
            "assertions": [{
                "earl:testSubject": {"url": "EPUB/x.xhtml"},
                "assertions": [
                    {"earl:test": {"dct:title": "a"}, "earl:result": {"earl:outcome": "earl:passed"}},
                    {"earl:test": {"dct:title": "b"}, "earl:result": {"earl:outcome": "earl:inapplicable"}}
                ]
            }]
        })
        .to_string();
        let raw: RawReport = serde_json::from_str(&body).expect("parse");
        let report = into_report(raw);
        assert_eq!(report.summary.errors, 0);
        assert_eq!(report.summary.warnings, 0);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn empty_report_is_valid() {
        let raw: RawReport = serde_json::from_str("{}").expect("parse");
        let report = into_report(raw);
        assert_eq!(report.summary.errors, 0);
        assert!(report.issues.is_empty());
    }
}
