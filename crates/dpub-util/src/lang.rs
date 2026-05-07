//! ISO 639 language-code normalisation.
//!
//! DAISY 2.02 metadata typically uses ISO 639-1 two-letter codes (`nl`,
//! `en`), but real-world books occasionally carry ISO 639-2/B (`dut`) or
//! 639-2/T (`nld`) variants instead. Whisper (and most modern tooling)
//! expects ISO 639-1. This module provides a single normaliser that maps
//! any recognised variant to its canonical two-letter form.

/// Normalise an ISO 639-1, 639-2/B or 639-2/T language code to its
/// canonical ISO 639-1 (two-letter) form. Case-insensitive.
///
/// Returns `None` for codes that are not in the lookup table.
///
/// ```
/// assert_eq!(dpub_util::lang::iso639_to_part1("dut"), Some("nl"));
/// assert_eq!(dpub_util::lang::iso639_to_part1("NL"),  Some("nl"));
/// assert_eq!(dpub_util::lang::iso639_to_part1("nld"), Some("nl"));
/// assert_eq!(dpub_util::lang::iso639_to_part1("xyz"), None);
/// ```
pub fn iso639_to_part1(code: &str) -> Option<&'static str> {
    // Lowercase + trim once; the match arms are all-lowercase.
    let code = code.trim();
    // Fast path: stack-allocated lowercase for short codes.
    let mut buf = [0u8; 8];
    if code.len() > buf.len() {
        return None;
    }
    for (i, b) in code.bytes().enumerate() {
        buf[i] = b.to_ascii_lowercase();
    }
    let lc = std::str::from_utf8(&buf[..code.len()]).ok()?;

    match lc {
        // Whisper-supported languages that appear in DAISY books.
        // Each group: part-1 | part-2/B | part-2/T (where they differ).
        "af" | "afr" => Some("af"),
        "ar" | "ara" => Some("ar"),
        "be" | "bel" => Some("be"),
        "bg" | "bul" => Some("bg"),
        "bn" | "ben" => Some("bn"),
        "ca" | "cat" => Some("ca"),
        "cs" | "cze" | "ces" => Some("cs"),
        "cy" | "wel" | "cym" => Some("cy"),
        "da" | "dan" => Some("da"),
        "de" | "ger" | "deu" => Some("de"),
        "el" | "gre" | "ell" => Some("el"),
        "en" | "eng" => Some("en"),
        "es" | "spa" => Some("es"),
        "et" | "est" => Some("et"),
        "eu" | "baq" | "eus" => Some("eu"),
        "fa" | "per" | "fas" => Some("fa"),
        "fi" | "fin" => Some("fi"),
        "fr" | "fre" | "fra" => Some("fr"),
        "gl" | "glg" => Some("gl"),
        "he" | "heb" => Some("he"),
        "hi" | "hin" => Some("hi"),
        "hr" | "hrv" => Some("hr"),
        "hu" | "hun" => Some("hu"),
        "hy" | "arm" | "hye" => Some("hy"),
        "id" | "ind" => Some("id"),
        "is" | "ice" | "isl" => Some("is"),
        "it" | "ita" => Some("it"),
        "ja" | "jpn" => Some("ja"),
        "ka" | "geo" | "kat" => Some("ka"),
        "kk" | "kaz" => Some("kk"),
        "ko" | "kor" => Some("ko"),
        "lt" | "lit" => Some("lt"),
        "lv" | "lav" => Some("lv"),
        "mk" | "mac" | "mkd" => Some("mk"),
        "mr" | "mar" => Some("mr"),
        "ms" | "may" | "msa" => Some("ms"),
        "ne" | "nep" => Some("ne"),
        "nl" | "dut" | "nld" => Some("nl"),
        "no" | "nor" => Some("no"),
        "pl" | "pol" => Some("pl"),
        "pt" | "por" => Some("pt"),
        "ro" | "rum" | "ron" => Some("ro"),
        "ru" | "rus" => Some("ru"),
        "sk" | "slo" | "slk" => Some("sk"),
        "sl" | "slv" => Some("sl"),
        "sr" | "srp" => Some("sr"),
        "sv" | "swe" => Some("sv"),
        "sw" | "swa" => Some("sw"),
        "ta" | "tam" => Some("ta"),
        "th" | "tha" => Some("th"),
        "tl" | "tgl" => Some("tl"),
        "tr" | "tur" => Some("tr"),
        "uk" | "ukr" => Some("uk"),
        "ur" | "urd" => Some("ur"),
        "vi" | "vie" => Some("vi"),
        "zh" | "chi" | "zho" => Some("zh"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part1_passes_through() {
        assert_eq!(iso639_to_part1("nl"), Some("nl"));
        assert_eq!(iso639_to_part1("en"), Some("en"));
        assert_eq!(iso639_to_part1("fr"), Some("fr"));
    }

    #[test]
    fn part2b_normalises() {
        assert_eq!(iso639_to_part1("dut"), Some("nl"));
        assert_eq!(iso639_to_part1("fre"), Some("fr"));
        assert_eq!(iso639_to_part1("ger"), Some("de"));
        assert_eq!(iso639_to_part1("cze"), Some("cs"));
    }

    #[test]
    fn part2t_normalises() {
        assert_eq!(iso639_to_part1("nld"), Some("nl"));
        assert_eq!(iso639_to_part1("fra"), Some("fr"));
        assert_eq!(iso639_to_part1("deu"), Some("de"));
        assert_eq!(iso639_to_part1("ces"), Some("cs"));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(iso639_to_part1("NL"), Some("nl"));
        assert_eq!(iso639_to_part1("DUT"), Some("nl"));
        assert_eq!(iso639_to_part1("Eng"), Some("en"));
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(iso639_to_part1("  nl "), Some("nl"));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(iso639_to_part1("xyz"), None);
        assert_eq!(iso639_to_part1(""), None);
        assert_eq!(iso639_to_part1("this-is-too-long-to-be-a-code"), None);
    }
}
