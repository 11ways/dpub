//! EPUB 3 validation via two complementary backends:
//!
//! - **[EPUBCheck]** — official Java tool from the W3C. Validates the EPUB
//!   *format* against the spec (manifest, spine, SMIL grammar, …).
//! - **[ACE]** — accessibility checker from the DAISY Consortium. Runs
//!   axe-core for WCAG conformance plus EPUB-specific accessibility
//!   checks (a11y metadata, alt text, page-break sources, …).
//!
//! Spec compliance ≠ accessibility; both matter under the European
//! Accessibility Act and comparable regimes.
//!
//! [EPUBCheck]: https://github.com/w3c/epubcheck
//! [ACE]: https://github.com/daisy/ace
//!
//! Entry points:
//!
//! - [`validate_epub`] — full validation. Returns a [`Report`] aggregating
//!   every backend that ran.
//! - [`epubcheck_available`] / [`ace_available`] — quick presence checks.
//!
//! Both backends are opt-in by way of `PATH` discovery: each runs only
//! when its CLI is available. Missing backends are reported as `None`
//! slots in [`Report`], never as errors.

mod ace;
mod epubcheck;
mod error;
mod report;

pub use ace::{ace_available, run_ace};
pub use epubcheck::{epubcheck_available, run_epubcheck};
pub use error::{Error, Result};
pub use report::{BackendReport, Issue, Report, Severity, Summary};

use std::path::Path;

/// Run every available validator against the given `.epub` file.
///
/// Backends not on `PATH` are skipped silently; their slots in the
/// [`Report`] are `None`. A backend that fails to run (spawn error,
/// unparseable output) propagates the error rather than disappearing.
pub fn validate_epub(epub_path: &Path) -> Result<Report> {
    let mut report = Report::default();
    if epubcheck_available() {
        report.epubcheck = Some(run_epubcheck(epub_path)?);
    }
    if ace_available() {
        report.ace = Some(run_ace(epub_path)?);
    }
    Ok(report)
}
