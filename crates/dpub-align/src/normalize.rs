//! Word normalisation for matching.
//!
//! Produces a "match key" by lowercasing and stripping punctuation.
//! The original surface form (with punctuation, capitalisation) is
//! preserved separately by callers and flows through to the output —
//! normalisation is *only* used as the diff-equality key.

/// Return the normalised match key for a word: Unicode-lowercased,
/// with leading/trailing punctuation stripped. Internal apostrophes
/// (`don't`, `c'est`) and hyphens (`well-known`) are preserved so
/// English/French/Dutch contractions and compounds stay intact.
pub fn normalise(word: &str) -> String {
    // Strip surrounding punctuation/quotes/brackets/whitespace.
    let trimmed = word.trim_matches(|c: char| {
        c.is_whitespace() || is_strippable_punct(c)
    });
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.to_lowercase()
}

fn is_strippable_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | '!' | '?' | '…'
            | '"' | '\u{201C}' | '\u{201D}'      // " " "
            | '\'' | '\u{2018}' | '\u{2019}'    // ' ' '
            | '(' | ')' | '[' | ']' | '{' | '}'
            | '«' | '»' | '‹' | '›'
            | '—' | '–'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases() {
        assert_eq!(normalise("Hello"), "hello");
        assert_eq!(normalise("WORLD"), "world");
    }

    #[test]
    fn strips_trailing_punctuation() {
        assert_eq!(normalise("wereld."), "wereld");
        assert_eq!(normalise("wereld,"), "wereld");
        assert_eq!(normalise("wereld!"), "wereld");
        assert_eq!(normalise("wereld?"), "wereld");
        assert_eq!(normalise("wereld..."), "wereld");
        assert_eq!(normalise("wereld…"), "wereld");
    }

    #[test]
    fn strips_brackets_and_quotes() {
        assert_eq!(normalise("(hello"), "hello");
        assert_eq!(normalise("hello)"), "hello");
        assert_eq!(normalise("\"quoted\""), "quoted");
        assert_eq!(normalise("'word'"), "word");
        assert_eq!(normalise("\u{201C}smart\u{201D}"), "smart");
    }

    #[test]
    fn preserves_internal_apostrophes() {
        assert_eq!(normalise("don't"), "don't");
        assert_eq!(normalise("c'est"), "c'est");
    }

    #[test]
    fn preserves_internal_hyphens() {
        assert_eq!(normalise("well-known"), "well-known");
        assert_eq!(normalise("co-op."), "co-op");
    }

    #[test]
    fn pure_punctuation_returns_empty() {
        assert_eq!(normalise("."), "");
        assert_eq!(normalise("..."), "");
        assert_eq!(normalise("\""), "");
        assert_eq!(normalise(""), "");
    }

    #[test]
    fn unicode_passes_through() {
        assert_eq!(normalise("café"), "café");
        assert_eq!(normalise("Antwerpen,"), "antwerpen");
    }
}
