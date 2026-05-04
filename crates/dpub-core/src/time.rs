//! SMIL 1.0 / DAISY 2.02 time-value parsing.
//!
//! SMIL 1.0 clock values come in three flavours:
//! - **Full clock value**: `HH:MM:SS(.fraction)?` — e.g. `01:23:45.678`
//! - **Partial clock value**: `MM:SS(.fraction)?` or `(MM):SS(.fraction)?`
//! - **Timecount value**: `<number>(h|min|s|ms)?` — e.g. `1.234s`, `45min`, `200ms`
//!
//! DAISY 2.02 wraps these in an `npt=` prefix on `clip-begin`/`clip-end`
//! attributes (e.g. `npt=12.345s`); we strip the prefix when present.

use std::num::ParseFloatError;

#[derive(Debug, thiserror::Error)]
pub enum TimeParseError {
    #[error("empty time value")]
    Empty,
    #[error("invalid time value: {0:?}")]
    Invalid(String),
    #[error("invalid number in time value: {0}")]
    InvalidNumber(#[from] ParseFloatError),
}

/// Parse a SMIL 1.0 clock value (with or without `npt=` prefix) into seconds.
///
/// ```
/// # use dpub_core::time::parse_clock_value;
/// assert!((parse_clock_value("npt=12.345s").unwrap() - 12.345).abs() < 1e-9);
/// assert!((parse_clock_value("1.5s").unwrap() - 1.5).abs() < 1e-9);
/// assert!((parse_clock_value("00:01:30").unwrap() - 90.0).abs() < 1e-9);
/// assert!((parse_clock_value("01:30").unwrap() - 90.0).abs() < 1e-9);
/// assert!((parse_clock_value("250ms").unwrap() - 0.25).abs() < 1e-9);
/// ```
pub fn parse_clock_value(input: &str) -> Result<f64, TimeParseError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(TimeParseError::Empty);
    }
    let s = s.strip_prefix("npt=").unwrap_or(s);

    if s.contains(':') {
        return parse_colon_form(s);
    }

    parse_timecount(s)
}

fn parse_colon_form(s: &str) -> Result<f64, TimeParseError> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [m, sec] => {
            let minutes: f64 = m.parse()?;
            let seconds: f64 = sec.parse()?;
            Ok(minutes * 60.0 + seconds)
        }
        [h, m, sec] => {
            let hours: f64 = h.parse()?;
            let minutes: f64 = m.parse()?;
            let seconds: f64 = sec.parse()?;
            Ok(hours * 3600.0 + minutes * 60.0 + seconds)
        }
        _ => Err(TimeParseError::Invalid(s.to_owned())),
    }
}

fn parse_timecount(s: &str) -> Result<f64, TimeParseError> {
    // Find the longest unit suffix.
    for (suffix, multiplier) in &[("ms", 0.001), ("h", 3600.0), ("min", 60.0), ("s", 1.0)] {
        if let Some(num) = s.strip_suffix(*suffix) {
            let n: f64 = num.parse()?;
            return Ok(n * multiplier);
        }
    }
    // No unit: SMIL says default is seconds.
    let n: f64 = s.parse()?;
    Ok(n)
}

/// Format a duration in seconds back to an `npt=` clock value compatible with
/// the original DAISY 2.02 SMIL files (three decimals, `s` suffix).
pub fn format_clock_value(seconds: f64) -> String {
    format!("npt={seconds:.3}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected ~{b}, got {a}");
    }

    #[test]
    fn npt_seconds() {
        approx(parse_clock_value("npt=0.000s").unwrap(), 0.0);
        approx(parse_clock_value("npt=12.345s").unwrap(), 12.345);
    }

    #[test]
    fn timecount_units() {
        approx(parse_clock_value("1.5s").unwrap(), 1.5);
        approx(parse_clock_value("250ms").unwrap(), 0.25);
        approx(parse_clock_value("2min").unwrap(), 120.0);
        approx(parse_clock_value("1h").unwrap(), 3600.0);
        approx(parse_clock_value("42").unwrap(), 42.0);
    }

    #[test]
    fn colon_forms() {
        approx(parse_clock_value("00:01:30").unwrap(), 90.0);
        approx(parse_clock_value("01:30").unwrap(), 90.0);
        approx(parse_clock_value("11:45:09").unwrap(), 42_309.0);
        approx(parse_clock_value("00:00:01.234").unwrap(), 1.234);
    }

    #[test]
    fn malformed() {
        assert!(parse_clock_value("").is_err());
        assert!(parse_clock_value("abc").is_err());
        assert!(parse_clock_value("1:2:3:4").is_err());
    }

    #[test]
    fn round_trip_through_npt() {
        let original = "npt=943.784s";
        let parsed = parse_clock_value(original).unwrap();
        let reformatted = format_clock_value(parsed);
        assert_eq!(reformatted, original);
    }
}
