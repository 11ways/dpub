//! EPUB 3 validation, currently via the official Java [EPUBCheck] tool
//! invoked as a subprocess.
//!
//! [EPUBCheck]: https://github.com/w3c/epubcheck
//!
//! Two entry points:
//!
//! - [`validate_epub`] — full validation. Returns a [`Report`] aggregating
//!   each backend that ran.
//! - [`epubcheck_available`] / [`ace_available`] — quick presence checks
//!   that callers can use to decide whether validation is even possible.
//!
//! ACE (DAISY's accessibility checker) integration is stubbed out for now —
//! it requires Node.js and a separate install, and most users will get more
//! value from EPUBCheck alone in v1.

mod epubcheck;
mod error;
mod report;

pub use epubcheck::{epubcheck_available, run_epubcheck};
pub use error::{Error, Result};
pub use report::{Issue, Report, Severity, Summary};

use std::path::Path;

/// Run all available validators against the given `.epub` file.
///
/// The current build only runs EPUBCheck. The function still returns a
/// [`Report`] with an `epubcheck` slot so callers don't have to special-case
/// "no backend ran"; an absent EPUBCheck is reported as a `None` slot, not
/// an error.
pub fn validate_epub(epub_path: &Path) -> Result<Report> {
    let mut report = Report::default();
    if epubcheck_available() {
        report.epubcheck = Some(run_epubcheck(epub_path)?);
    }
    Ok(report)
}

/// Stubbed: ACE is not yet wired up. Always returns `false`.
pub fn ace_available() -> bool {
    false
}
