use serde::Serialize;

/// Aggregated validation outcome from every backend that ran.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub epubcheck: Option<BackendReport>,
    pub ace: Option<BackendReport>,
}

impl Report {
    /// `true` if no backend produced an error (or fatal). Warnings and infos
    /// are allowed.
    pub fn is_clean(&self) -> bool {
        let backend_clean = |r: &BackendReport| r.summary.errors == 0 && r.summary.fatals == 0;
        self.epubcheck.as_ref().is_none_or(backend_clean)
            && self.ace.as_ref().is_none_or(backend_clean)
    }
}

/// Result of a single backend (EPUBCheck or ACE).
#[derive(Debug, Clone, Serialize)]
pub struct BackendReport {
    pub backend: String,
    pub version: Option<String>,
    pub summary: Summary,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Summary {
    pub fatals: u32,
    pub errors: u32,
    pub warnings: u32,
    pub infos: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Fatal,
    Error,
    Warning,
    Info,
    Usage,
    Suppressed,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Fatal => "FATAL",
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
            Severity::Usage => "USAGE",
            Severity::Suppressed => "SUPPRESSED",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub id: Option<String>,
    pub message: String,
    pub location: Option<String>,
}
