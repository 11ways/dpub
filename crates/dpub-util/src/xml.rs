//! XML/XHTML text and attribute escaping.
//!
//! Both functions return [`Cow::Borrowed`] when the input contains no
//! characters that need escaping — common for typed metadata like
//! identifiers or UUIDs — so well-formed input does not allocate.
//!
//! ```
//! use std::borrow::Cow;
//! use dpub_util::xml::{escape_text, escape_attr};
//!
//! assert!(matches!(escape_text("plain"), Cow::Borrowed("plain")));
//! assert_eq!(escape_text("a < b"), "a &lt; b");
//! assert_eq!(escape_attr(r#"a "quoted" b"#), "a &quot;quoted&quot; b");
//! ```

use std::borrow::Cow;

/// Escape `&`, `<`, and `>` for use in XML/XHTML element text content.
pub fn escape_text(s: &str) -> Cow<'_, str> {
    escape(s, false)
}

/// Escape `&`, `<`, `>`, and `"` for use inside double-quoted attribute
/// values. (We always emit `"`-quoted attributes, so single quotes pass
/// through unchanged.)
pub fn escape_attr(s: &str) -> Cow<'_, str> {
    escape(s, true)
}

fn escape(s: &str, escape_quote: bool) -> Cow<'_, str> {
    let needs_escape = |c: char| matches!(c, '&' | '<' | '>') || (escape_quote && c == '"');

    if !s.contains(needs_escape) {
        return Cow::Borrowed(s);
    }

    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if escape_quote => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_borrows() {
        assert!(matches!(escape_text("plain"), Cow::Borrowed("plain")));
        assert!(matches!(escape_attr("plain"), Cow::Borrowed("plain")));
        assert!(matches!(escape_text(""), Cow::Borrowed("")));
    }

    #[test]
    fn text_escapes_amp_lt_gt_only() {
        assert_eq!(escape_text("a & b"), "a &amp; b");
        assert_eq!(escape_text("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
        assert_eq!(escape_text(r#"a "b" c"#), r#"a "b" c"#); // " stays as-is in text
    }

    #[test]
    fn attr_escapes_quote_too() {
        assert_eq!(escape_attr(r#"a "b" c"#), "a &quot;b&quot; c");
        assert_eq!(escape_attr("a&b"), "a&amp;b");
    }

    #[test]
    fn no_double_escape() {
        // Already-escaped entities are escaped again — the function is for
        // raw text, not for embedding pre-escaped HTML.
        assert_eq!(escape_text("&amp;"), "&amp;amp;");
    }

    #[test]
    fn unicode_passes_through() {
        assert_eq!(escape_text("café — ☕"), "café — ☕");
        assert!(matches!(escape_text("café"), Cow::Borrowed("café")));
    }
}
