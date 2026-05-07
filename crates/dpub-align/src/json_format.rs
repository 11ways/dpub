//! Adapter for the structured-JSON ground truth format.
//!
//! Schema (permissive — unknown fields are ignored):
//!
//! ```json
//! {
//!   "content": [
//!     { "chapter-title": "...", "chapter-content": "..." },
//!     ...
//!   ]
//! }
//! ```
//!
//! Each entry becomes one section. The title is matched against the
//! DAISY NCC heading via the existing fuzzy matcher; the content
//! becomes the section body. Single newlines in the content are
//! treated as paragraph breaks (DAISY-friendly default — most book
//! exporters split paragraphs that way).

use serde::Deserialize;

/// Returns `true` when `raw` looks like our JSON format (first
/// non-whitespace char is `{`). Used to dispatch between the JSON
/// parser and the plain-text/markdown path.
pub fn looks_like_json(raw: &str) -> bool {
    raw.trim_start().starts_with('{')
}

/// Convert a JSON document conforming to the chapter-array schema
/// into the markdown-style ground-truth text the rest of `dpub-align`
/// already consumes. On any parse error returns the input unchanged
/// so the caller can fall through to the plain-text path.
pub fn convert_to_markdown(raw: &str) -> String {
    match serde_json::from_str::<Document>(raw) {
        Ok(doc) => render(&doc),
        Err(e) => {
            tracing::warn!("ground truth: JSON parse failed ({e}); falling back to plain-text path");
            raw.to_owned()
        }
    }
}

#[derive(Debug, Deserialize)]
struct Document {
    #[serde(default)]
    content: Vec<Chapter>,
}

#[derive(Debug, Deserialize)]
struct Chapter {
    #[serde(rename = "chapter-title", default)]
    title: Option<String>,
    #[serde(rename = "chapter-content", default)]
    content: Option<String>,
}

fn render(doc: &Document) -> String {
    // Bulk format: no chapter object carries a title — the whole book
    // is in one (or more) `chapter-content` blobs with section titles
    // encoded inline (typically as ALL-CAPS short lines). Concatenate
    // the bodies and pass through as plain text so the existing
    // line-by-line heading detector picks up the inline titles via
    // fuzzy matching.
    let any_titled = doc.content.iter().any(|c| {
        c.title
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
    });
    if !any_titled {
        let mut body = String::new();
        for ch in &doc.content {
            if let Some(content) = &ch.content {
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str(content.trim());
            }
        }
        return normalize_body_preserving_lines(&body);
    }

    let mut out = String::with_capacity(doc.content.iter().map(|c| {
        c.title.as_deref().map_or(0, str::len) + c.content.as_deref().map_or(0, str::len) + 8
    }).sum());

    for chapter in &doc.content {
        let body_raw = chapter.content.as_deref().unwrap_or("").trim();
        let title = chapter
            .title
            .as_deref()
            .map(normalize_title)
            .filter(|t| !t.is_empty());

        // Skip entries with neither title nor content — nothing useful
        // to align against.
        if title.is_none() && body_raw.is_empty() {
            continue;
        }

        // Emit a markdown H1 so the existing splitter picks it up.
        // Untitled entries (typically the first cover/title-page item)
        // get a synthetic placeholder so they still count as a section
        // boundary; matchers will simply fail to find them in the NCC,
        // which is the correct outcome.
        let heading = title.unwrap_or_else(|| body_raw
            .lines()
            .next()
            .unwrap_or("untitled")
            .chars()
            .take(80)
            .collect::<String>());

        out.push_str("# ");
        out.push_str(heading.trim());
        out.push_str("\n\n");

        let body = normalize_body(body_raw);
        out.push_str(&body);
        out.push_str("\n\n");
    }

    out
}

/// Normalise a chapter title that may contain literal newlines or
/// non-breaking spaces (`\u{a0}`). Reading systems display titles on
/// one line; the NCC heading we match against is also a single line.
fn normalize_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = true;
    for c in s.chars() {
        let is_space = c == '\n' || c == '\r' || c == '\t' || c == '\u{a0}' || c == ' ';
        if is_space {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    out.trim().to_owned()
}

/// Convert the JSON's `chapter-content` into the paragraph-aware
/// format the rest of the pipeline expects (paragraphs separated by
/// blank lines). Heuristic:
/// - Treat single `\n` as a paragraph break (the format uses single
///   newlines between paragraphs).
/// - Collapse runs of newlines into a single paragraph break.
/// - Replace non-breaking spaces with regular spaces (better word
///   matching: `nieuwe\u{a0}avonturen` matches Whisper's `nieuwe
///   avonturen`).
/// Normalise a bulk-format body without merging lines: inline
/// chapter titles (ALL-CAPS short lines preceded by blank lines) must
/// stay on their own lines so [`section_split`] can detect them.
/// Only character-level normalisations are applied (NBSP → space).
fn normalize_body_preserving_lines(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\u{a0}' || c == '\u{2009}' || c == '\u{200a}' { ' ' } else { c })
        .collect()
}

fn normalize_body(s: &str) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    for raw_para in s.split('\n') {
        let trimmed = raw_para.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Replace NBSPs and other Unicode spaces with a regular space.
        let normalised: String = trimmed
            .chars()
            .map(|c| if c == '\u{a0}' || c == '\u{2009}' || c == '\u{200a}' { ' ' } else { c })
            .collect();
        paragraphs.push(normalised);
    }
    paragraphs.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_json_input() {
        assert!(looks_like_json("{\"content\": []}"));
        assert!(looks_like_json("  \n  { ... }"));
        assert!(!looks_like_json("# Heading\nbody"));
        assert!(!looks_like_json("plain text"));
        assert!(!looks_like_json(""));
    }

    #[test]
    fn converts_basic_document() {
        let json = r#"{
            "content": [
                {"chapter-title": "Chapter 1", "chapter-content": "First paragraph.\nSecond paragraph."},
                {"chapter-title": "Chapter 2", "chapter-content": "Body of two."}
            ]
        }"#;
        let md = convert_to_markdown(json);
        assert!(md.contains("# Chapter 1"));
        assert!(md.contains("# Chapter 2"));
        // Paragraphs separated by blank lines (i.e. \n\n).
        assert!(md.contains("First paragraph.\n\nSecond paragraph."));
    }

    #[test]
    fn collapses_multiline_title() {
        let json = r#"{"content":[
            {"chapter-title": "Hoera!\nNieuwe avonturen", "chapter-content": "Body."}
        ]}"#;
        let md = convert_to_markdown(json);
        assert!(md.contains("# Hoera! Nieuwe avonturen"));
    }

    #[test]
    fn replaces_nbsp_in_title_and_body() {
        let json = "{\"content\":[{\"chapter-title\":\"de\u{a0}cavia\",\"chapter-content\":\"woord\u{a0}met nbsp.\"}]}";
        let md = convert_to_markdown(json);
        assert!(md.contains("# de cavia"));
        assert!(md.contains("woord met nbsp."));
    }

    #[test]
    fn skips_entry_with_no_useful_content() {
        let json = r#"{"content":[
            {"chapter-content": ""},
            {"chapter-title": "Real chapter", "chapter-content": "Body."}
        ]}"#;
        let md = convert_to_markdown(json);
        // Only one heading.
        assert_eq!(md.matches("# ").count(), 1);
        assert!(md.contains("# Real chapter"));
    }

    #[test]
    fn ignores_extra_top_level_fields() {
        let json = r#"{
            "title": "Book Title",
            "language": "nl",
            "extraction_time_ms": 12345,
            "total_chars_count": 999,
            "content": [
                {"chapter-title": "Only", "chapter-content": "Body."}
            ]
        }"#;
        let md = convert_to_markdown(json);
        assert!(md.contains("# Only"));
        assert!(md.contains("Body."));
    }

    #[test]
    fn ignores_extra_chapter_fields() {
        let json = r#"{"content":[
            {"chapter-title": "T", "chapter-content": "B", "chars_count": 1, "word_count": 1, "anything-else": null}
        ]}"#;
        let md = convert_to_markdown(json);
        assert!(md.contains("# T"));
        assert!(md.contains("B"));
    }

    #[test]
    fn malformed_json_falls_through_unchanged() {
        let raw = "{not json";
        assert_eq!(convert_to_markdown(raw), raw);
    }

    /// Smoke test against a real fullbook.json (when present on disk).
    /// Gated on the env var so CI without the file passes. The fixture
    /// may be either format (structured or bulk); we just assert that
    /// parsing produces a non-empty result.
    #[test]
    fn parses_real_fullbook_json() {
        let Ok(path) = std::env::var("DPUB_TEST_GROUND_TRUTH_JSON") else {
            return;
        };
        let raw = std::fs::read_to_string(&path).expect("read fixture");
        assert!(looks_like_json(&raw));
        let md = convert_to_markdown(&raw);
        assert!(!md.is_empty());
    }

    /// Integration: parse the bulk-format fullbook.json and verify the
    /// inline ALL-CAPS chapter titles can be matched against a sample
    /// of expected DAISY headings via the public splitter API.
    #[test]
    fn bulk_format_finds_inline_chapter_titles() {
        let Ok(path) = std::env::var("DPUB_TEST_GROUND_TRUTH_JSON") else {
            return;
        };
        let raw = std::fs::read_to_string(&path).expect("read fixture");
        // Skip if it's not the bulk format we want to exercise.
        let doc: Document = serde_json::from_str(&raw).expect("valid json");
        let any_titled = doc.content.iter().any(|c| {
            c.title.as_deref().is_some_and(|t| !t.trim().is_empty())
        });
        if any_titled {
            return; // structured format — different test.
        }
        let md = convert_to_markdown(&raw);
        // Sample of titles known to live inline in this fixture
        // (copied from the DAISY filenames of the matching book).
        let ncc: &[(&str, usize)] = &[
            ("Hh", 0),
            ("Opletten", 1),
            ("Nierstenen", 2),
            ("Ping", 3),
            ("Opvangbakje", 4),
        ];
        let sections = crate::split_into_sections(&md, ncc);
        // The fuzzy matcher should find at least 3 of the 5 — this is
        // a loose threshold so the test isn't brittle if NCC formatting
        // changes the matcher's preferences.
        assert!(
            sections.len() >= 3,
            "expected ≥3 of 5 sample headings to match, got {}",
            sections.len()
        );
    }

    #[test]
    fn bulk_format_passes_body_through_as_plain_text() {
        // Single-chapter document with the whole book in one blob and
        // ALL-CAPS chapter titles inline. We must NOT prepend a `# `
        // wrapper — the inline titles need to remain plain lines so
        // section_split's plain-text path can fuzzy-match them.
        let json = r#"{"content":[{"chapter-content":
"Cover blurb.\n\nHÈHÈ\nFirst chapter body.\n\nOPLETTEN\nSecond chapter body."}]}"#;
        let md = convert_to_markdown(json);
        // No markdown headings emitted.
        assert!(!md.contains("# "));
        // Inline titles preserved as their own lines.
        assert!(md.contains("\nHÈHÈ\n"));
        assert!(md.contains("\nOPLETTEN\n"));
    }

    #[test]
    fn untitled_first_entry_gets_placeholder() {
        // Real-world case: the first entry has no `chapter-title`.
        let json = r#"{"content":[
            {"chapter-content": "De verwarde cavia"},
            {"chapter-title": "Hoofdstuk 1", "chapter-content": "Body."}
        ]}"#;
        let md = convert_to_markdown(json);
        // Two headings: one synthetic from body, one explicit.
        assert_eq!(md.matches("# ").count(), 2);
        assert!(md.contains("# De verwarde cavia"));
        assert!(md.contains("# Hoofdstuk 1"));
    }
}
