//! Best-effort book-cover lookup against the [Open Library] covers API.
//!
//! [Open Library]: https://openlibrary.org/dev/docs/api/covers
//!
//! Two-step flow:
//!
//! 1. Search `openlibrary.org/search.json` by ISBN if the DAISY
//!    metadata's `dc:identifier` looks ISBN-shaped, otherwise by
//!    title + author.
//! 2. If a hit looks plausible (language match + author last-name
//!    overlap), fetch the cover image from `covers.openlibrary.org`
//!    at the medium size and return the bytes.
//!
//! Network failures, ambiguous matches, or zero-hit searches resolve
//! to `Ok(None)` rather than `Err(...)`. Missing covers are normal
//! for older or non-commercial DAISY books, and `--auto-cover` is
//! best-effort — the caller can fall back gracefully.
//!
//! Privacy note: each lookup sends the book's title, author, and (if
//! present) ISBN to a third party. Callers should keep the feature
//! opt-in for that reason.

use std::io::{Read, Write};
use std::time::Duration;

use serde::Deserialize;

mod error;

pub use error::{Error, Result};

const SEARCH_URL: &str = "https://openlibrary.org/search.json";
const COVERS_URL: &str = "https://covers.openlibrary.org/b";
const USER_AGENT_BASE: &str = "dpub";
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;

/// Build a fresh `ureq::Agent` with dpub's standard configuration:
/// 8-second timeout, identifying User-Agent. Callers that need a
/// generic HTTP-download path (Whisper model fetch, etc.) can use
/// this directly via [`download_to_writer`].
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .user_agent(&format!(
            "{USER_AGENT_BASE}/{} (+https://github.com/11ways/dpub)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
}

/// Stream a URL into `dest`, calling `on_progress(bytes_so_far,
/// content_length_or_zero)` periodically. Used by the Whisper model
/// downloader. No `Range` resumption in v1 — a partial file is
/// truncated on retry.
///
/// Returns the total number of bytes written.
pub fn download_to_writer<W: Write>(
    agent: &ureq::Agent,
    url: &str,
    dest: &mut W,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let resp = agent.get(url).call()?;
    let content_length: u64 = resp
        .header("content-length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    on_progress(0, content_length);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dest.write_all(&buf[..n])?;
        total += n as u64;
        on_progress(total, content_length);
    }
    Ok(total)
}

/// Identifying bits we have for a book — typically derived from
/// DAISY 2.02 NCC metadata (`dc:title`, `dc:creator`, `dc:language`,
/// `dc:identifier`).
#[derive(Debug, Clone)]
pub struct LookupHints<'a> {
    pub title: &'a str,
    pub creator: Option<&'a str>,
    pub language: Option<&'a str>,
    pub identifier: Option<&'a str>,
}

/// One cover image: bytes + media type, ready to embed in the
/// `epub3-writer` `CoverImage` model.
#[derive(Debug, Clone)]
pub struct FetchedCover {
    pub bytes: Vec<u8>,
    pub media_type: String,
    /// Human-readable description of *why* this cover was selected,
    /// useful for log lines so an operator can verify.
    pub provenance: String,
}

/// Look up a cover for the given hints. Returns `Ok(None)` when the
/// search returns no plausible match; returns `Err` only on hard
/// failures (transport errors, malformed JSON).
pub fn lookup_cover(hints: &LookupHints<'_>) -> Result<Option<FetchedCover>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(&format!(
            "{USER_AGENT_BASE}/{} (+https://github.com/11ways/dpub)",
            env!("CARGO_PKG_VERSION")
        ))
        .build();

    let candidate = match try_isbn(&agent, hints)? {
        Some(c) => Some(c),
        None => try_title_creator(&agent, hints)?,
    };
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    fetch_cover(&agent, &candidate).map(Some)
}

#[derive(Debug)]
struct Candidate {
    cover_id: i64,
    edition_key: Option<String>,
    title: String,
    author: Option<String>,
    by: &'static str, // human label of which lookup path picked it
}

fn try_isbn(agent: &ureq::Agent, hints: &LookupHints<'_>) -> Result<Option<Candidate>> {
    let Some(isbn) = hints.identifier.and_then(extract_isbn) else {
        return Ok(None);
    };
    let resp: SearchResponse = agent
        .get(SEARCH_URL)
        .query("isbn", &isbn)
        .query("limit", "5")
        .call()?
        .into_json()?;
    Ok(pick(&resp, hints, "isbn"))
}

fn try_title_creator(agent: &ureq::Agent, hints: &LookupHints<'_>) -> Result<Option<Candidate>> {
    let mut req = agent
        .get(SEARCH_URL)
        .query("title", hints.title)
        .query("limit", "5");
    if let Some(creator) = hints.creator {
        req = req.query("author", creator);
    }
    let resp: SearchResponse = req.call()?.into_json()?;
    Ok(pick(&resp, hints, "title+author"))
}

/// Pick the first hit whose language matches the hints (when given) and
/// whose author overlaps the requested creator's last name. Returns
/// `None` if no hit clears the bar.
fn pick(resp: &SearchResponse, hints: &LookupHints<'_>, by: &'static str) -> Option<Candidate> {
    let want_lang = hints.language.map(str::to_lowercase);
    let want_lastname = hints
        .creator
        .map(last_name)
        .map(str::to_lowercase);

    for doc in &resp.docs {
        let Some(cover_id) = doc.cover_i else {
            continue;
        };
        if let Some(lang) = &want_lang
            && !doc.language.iter().any(|l| l.eq_ignore_ascii_case(lang))
        {
            continue;
        }
        if let Some(last) = &want_lastname
            && !doc
                .author_name
                .iter()
                .any(|a| a.to_lowercase().contains(last))
        {
            continue;
        }
        return Some(Candidate {
            cover_id,
            edition_key: doc.cover_edition_key.clone(),
            title: doc.title.clone().unwrap_or_default(),
            author: doc.author_name.first().cloned(),
            by,
        });
    }
    None
}

fn fetch_cover(agent: &ureq::Agent, candidate: &Candidate) -> Result<FetchedCover> {
    let url = if let Some(olid) = &candidate.edition_key {
        format!("{COVERS_URL}/olid/{olid}-L.jpg")
    } else {
        format!("{COVERS_URL}/id/{}-L.jpg", candidate.cover_id)
    };
    let resp = agent.get(&url).call()?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_COVER_BYTES as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() < 64 {
        // Open Library returns a 1×1 transparent placeholder when no
        // cover is available; treat anything tiny as "no cover".
        return Err(Error::NoCover);
    }
    let media_type = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else {
        return Err(Error::UnsupportedFormat);
    };
    let author_label = candidate.author.as_deref().unwrap_or("?");
    let provenance = format!(
        "Open Library [{by}] cover_i={id} \"{title}\" by {author}",
        by = candidate.by,
        id = candidate.cover_id,
        title = candidate.title,
        author = author_label,
    );
    Ok(FetchedCover {
        bytes,
        media_type: media_type.into(),
        provenance,
    })
}

/// Strip whitespace and hyphens from `id` and check if it's 10 or
/// 13 ASCII digits (with the last char of an ISBN-10 optionally
/// being `X`). DAISY identifiers are sometimes bare integers like
/// "5485" — those are *not* ISBNs and shouldn't be treated as such.
fn extract_isbn(id: &str) -> Option<String> {
    let cleaned: String = id
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    let len = cleaned.len();
    if len != 10 && len != 13 {
        return None;
    }
    let valid = cleaned
        .chars()
        .enumerate()
        .all(|(i, c)| c.is_ascii_digit() || (i + 1 == len && c == 'X'));
    if !valid {
        return None;
    }
    Some(cleaned)
}

/// Last whitespace-separated token of `name`. "De Ceuleneer Geertje"
/// → "Geertje", which is not always the surname for Dutch names but
/// is a reasonable approximation for the overlap heuristic. The
/// goal is to reject obvious mismatches, not to be perfectly
/// linguistically correct.
fn last_name(name: &str) -> &str {
    name.split_whitespace().next_back().unwrap_or(name)
}

#[derive(Deserialize, Debug)]
struct SearchResponse {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Deserialize, Debug)]
struct SearchDoc {
    title: Option<String>,
    cover_i: Option<i64>,
    cover_edition_key: Option<String>,
    #[serde(default)]
    language: Vec<String>,
    #[serde(default)]
    author_name: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_isbn_accepts_isbn13() {
        assert_eq!(
            extract_isbn("978-0-306-40615-7").as_deref(),
            Some("9780306406157")
        );
    }

    #[test]
    fn extract_isbn_accepts_isbn10_with_x() {
        assert_eq!(extract_isbn("0-306-40615-X").as_deref(), Some("030640615X"));
    }

    #[test]
    fn extract_isbn_rejects_bare_integer() {
        assert!(extract_isbn("5485").is_none());
    }

    #[test]
    fn extract_isbn_rejects_garbage() {
        assert!(extract_isbn("hello world").is_none());
    }

    #[test]
    fn last_name_basic() {
        assert_eq!(last_name("Geertje De Ceuleneer"), "Ceuleneer");
        assert_eq!(last_name("Austen, Jane"), "Jane");
        assert_eq!(last_name("Madonna"), "Madonna");
    }

    #[test]
    fn pick_rejects_language_mismatch() {
        let resp = SearchResponse {
            docs: vec![SearchDoc {
                title: Some("Some Book".into()),
                cover_i: Some(123),
                cover_edition_key: None,
                language: vec!["eng".into()],
                author_name: vec!["Geertje De Ceuleneer".into()],
            }],
        };
        let hints = LookupHints {
            title: "Some Book",
            creator: Some("Geertje De Ceuleneer"),
            language: Some("nl"),
            identifier: None,
        };
        assert!(pick(&resp, &hints, "test").is_none());
    }

    #[test]
    fn pick_accepts_when_everything_lines_up() {
        let resp = SearchResponse {
            docs: vec![SearchDoc {
                title: Some("Ontmoetingen in het donker".into()),
                cover_i: Some(99),
                cover_edition_key: Some("OL12345M".into()),
                language: vec!["nl".into()],
                author_name: vec!["Geertje De Ceuleneer".into()],
            }],
        };
        let hints = LookupHints {
            title: "Ontmoetingen in het donker",
            creator: Some("De Ceuleneer Geertje"),
            language: Some("nl"),
            identifier: None,
        };
        let got = pick(&resp, &hints, "test").expect("matched");
        assert_eq!(got.cover_id, 99);
        assert_eq!(got.edition_key.as_deref(), Some("OL12345M"));
    }

    #[test]
    fn pick_skips_docs_without_cover() {
        let resp = SearchResponse {
            docs: vec![
                SearchDoc {
                    title: Some("a".into()),
                    cover_i: None,
                    cover_edition_key: None,
                    language: vec!["nl".into()],
                    author_name: vec!["X".into()],
                },
                SearchDoc {
                    title: Some("b".into()),
                    cover_i: Some(7),
                    cover_edition_key: None,
                    language: vec!["nl".into()],
                    author_name: vec!["X".into()],
                },
            ],
        };
        let hints = LookupHints {
            title: "a",
            creator: Some("X"),
            language: Some("nl"),
            identifier: None,
        };
        assert_eq!(pick(&resp, &hints, "test").map(|c| c.cover_id), Some(7));
    }
}
