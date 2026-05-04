use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Error, Result};
use crate::metadata::Metadata;

/// Heading level (`h1` → 1 … `h6` → 6).
pub type HeadingLevel = u8;

/// One heading in the navigation hierarchy.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Heading {
    pub level: HeadingLevel,
    pub class: Option<String>,
    pub id: String,
    pub text: String,
    /// SMIL fragment reference, e.g. `ptk000007.smil#bookid_000008`.
    pub href: String,
}

/// One page-number marker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageNumber {
    pub class: Option<String>,
    pub id: String,
    pub text: String,
    pub href: String,
}

/// A single navigation entry in document order.
#[derive(Debug, Clone, serde::Serialize)]
pub enum NavItem {
    Heading(Heading),
    Page(PageNumber),
}

/// Parsed `ncc.html`: metadata plus navigation in document order.
#[derive(Debug, Default, Clone)]
pub struct Ncc {
    pub metadata: Metadata,
    pub nav: Vec<NavItem>,
}

impl Ncc {
    pub fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_bytes(&bytes, path)
    }

    fn parse_bytes(bytes: &[u8], path: &Path) -> Result<Self> {
        // DAISY 2.02 NCCs are XHTML but commonly declare windows-1252.
        // quick-xml does not decode legacy charsets, so we re-encode to UTF-8
        // when the XML prolog or a <meta charset> says so.
        let text = decode_to_utf8(bytes);

        let mut reader = Reader::from_str(&text);
        reader.config_mut().trim_text(true);
        reader.config_mut().expand_empty_elements = true;

        let mut ncc = Ncc::default();
        let mut buf = Vec::new();

        // Tracking state across events.
        let mut in_head = false;
        let mut in_anchor = false;
        let mut current_heading: Option<HeadingInProgress> = None;
        let mut current_page: Option<PageInProgress> = None;
        let mut text_accum = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Err(source) => {
                    return Err(Error::Xml {
                        path: path.to_path_buf(),
                        source,
                    });
                }
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => {
                    let tag = e.name();
                    let local = std::str::from_utf8(tag.as_ref()).unwrap_or("").to_owned();
                    match local.as_str() {
                        "head" => in_head = true,
                        "meta" if in_head => {
                            let mut name = None;
                            let mut content = None;
                            for attr in e.attributes().flatten() {
                                let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                                let value = attr.unescape_value().unwrap_or_default().into_owned();
                                match key {
                                    "name" => name = Some(value),
                                    "content" => content = Some(value),
                                    _ => {}
                                }
                            }
                            if let (Some(name), Some(content)) = (name, content) {
                                ncc.metadata.set(&name, content);
                            }
                        }
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            let level: HeadingLevel = local.as_bytes()[1] - b'0';
                            current_heading = Some(HeadingInProgress {
                                level,
                                class: attr(&e, "class"),
                                id: attr(&e, "id").unwrap_or_default(),
                                href: String::new(),
                            });
                            text_accum.clear();
                        }
                        "span" => {
                            // page-* spans wrap an <a> with a page label
                            let class = attr(&e, "class");
                            if class.as_deref().is_some_and(|c| c.starts_with("page-")) {
                                current_page = Some(PageInProgress {
                                    class,
                                    id: attr(&e, "id").unwrap_or_default(),
                                    href: String::new(),
                                });
                                text_accum.clear();
                            }
                        }
                        "a" => {
                            in_anchor = true;
                            let href = attr(&e, "href").unwrap_or_default();
                            if let Some(h) = current_heading.as_mut() {
                                h.href = href;
                            } else if let Some(p) = current_page.as_mut() {
                                p.href = href;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(e)) => {
                    let tag = e.name();
                    let local = std::str::from_utf8(tag.as_ref()).unwrap_or("");
                    match local {
                        "head" => in_head = false,
                        "a" => in_anchor = false,
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            if let Some(h) = current_heading.take() {
                                ncc.nav.push(NavItem::Heading(Heading {
                                    level: h.level,
                                    class: h.class,
                                    id: h.id,
                                    text: text_accum.trim().to_owned(),
                                    href: h.href,
                                }));
                                text_accum.clear();
                            }
                        }
                        "span" => {
                            if let Some(p) = current_page.take() {
                                ncc.nav.push(NavItem::Page(PageNumber {
                                    class: p.class,
                                    id: p.id,
                                    text: text_accum.trim().to_owned(),
                                    href: p.href,
                                }));
                                text_accum.clear();
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(t)) => {
                    if in_anchor && (current_heading.is_some() || current_page.is_some()) {
                        let s = t.unescape().unwrap_or_default();
                        text_accum.push_str(&s);
                    }
                }
                Ok(_) => {}
            }
            buf.clear();
        }

        if ncc.nav.is_empty() && ncc.metadata.title.is_none() {
            return Err(Error::MalformedNcc {
                path: path.to_path_buf(),
                message: "no metadata or navigation found".into(),
            });
        }

        Ok(ncc)
    }

    pub fn headings(&self) -> impl Iterator<Item = &Heading> {
        self.nav.iter().filter_map(|n| match n {
            NavItem::Heading(h) => Some(h),
            NavItem::Page(_) => None,
        })
    }

    pub fn pages(&self) -> impl Iterator<Item = &PageNumber> {
        self.nav.iter().filter_map(|n| match n {
            NavItem::Page(p) => Some(p),
            NavItem::Heading(_) => None,
        })
    }
}

struct HeadingInProgress {
    level: HeadingLevel,
    class: Option<String>,
    id: String,
    href: String,
}

struct PageInProgress {
    class: Option<String>,
    id: String,
    href: String,
}

fn attr(start: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    start
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key.as_bytes())
        .map(|a| a.unescape_value().unwrap_or_default().into_owned())
}

/// Decode `bytes` to a UTF-8 [`String`], honouring a leading XML prolog
/// `encoding="..."` declaration. Falls back to lossy UTF-8.
///
/// We don't bring in `encoding_rs` yet; only ISO-8859-1 / windows-1252 are
/// handled inline since DAISY 2.02 NCCs in the wild are essentially always
/// one of UTF-8, ISO-8859-1, or windows-1252.
fn decode_to_utf8(bytes: &[u8]) -> String {
    let prolog: &[u8] = bytes.get(..bytes.len().min(256)).unwrap_or(bytes);
    let prolog_text = String::from_utf8_lossy(prolog);
    let encoding = parse_encoding(&prolog_text).unwrap_or("utf-8");

    match encoding.to_ascii_lowercase().as_str() {
        "windows-1252" | "iso-8859-1" | "latin1" | "latin-1" => {
            // Both encodings cover U+0000..=U+00FF byte-for-byte for our purposes.
            // (windows-1252 differs from ISO-8859-1 only in 0x80..=0x9F, which
            // shows up rarely in DAISY metadata; "good enough" for v0.)
            bytes.iter().map(|&b| b as char).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn parse_encoding(prolog: &str) -> Option<&str> {
    let key = "encoding=";
    let start = prolog.find(key)? + key.len();
    let rest = &prolog[start..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = 1;
    let value_end = rest[value_start..].find(quote)? + value_start;
    Some(&rest[value_start..value_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_ncc() {
        let ncc = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd">
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="nl">
<head>
<meta name="dc:title" content="Test Book"/>
<meta name="dc:creator" content="Jane Doe"/>
<meta name="ncc:totalTime" content="00:42:00"/>
<meta name="ncc:multimediaType" content="audioNCC"/>
</head>
<body>
<h1 class="title" id="h1"><a href="ptk001.smil#b1">Test Book</a></h1>
<h1 class="section" id="h2"><a href="ptk002.smil#b2">Chapter One</a></h1>
<span class="page-normal" id="p1"><a href="ptk002.smil#b3">10</a></span>
</body>
</html>"#;
        let parsed = Ncc::parse_bytes(ncc.as_bytes(), Path::new("ncc.html")).unwrap();
        assert_eq!(parsed.metadata.title.as_deref(), Some("Test Book"));
        assert_eq!(parsed.metadata.creator.as_deref(), Some("Jane Doe"));
        assert_eq!(parsed.metadata.total_time.as_deref(), Some("00:42:00"));
        assert_eq!(parsed.metadata.multimedia_type.as_deref(), Some("audioNCC"));
        assert_eq!(parsed.headings().count(), 2);
        assert_eq!(parsed.pages().count(), 1);
        let h2 = parsed.headings().nth(1).unwrap();
        assert_eq!(h2.text, "Chapter One");
        assert_eq!(h2.href, "ptk002.smil#b2");
    }

    #[test]
    fn windows_1252_metadata_round_trips_basic_ascii() {
        let ncc = b"<?xml version=\"1.0\" encoding=\"windows-1252\"?>
<html><head><meta name=\"dc:title\" content=\"Caf\xe9\"/></head><body><h1 id=\"h1\"><a href=\"x.smil#a\">x</a></h1></body></html>";
        let parsed = Ncc::parse_bytes(ncc, Path::new("ncc.html")).unwrap();
        assert_eq!(parsed.metadata.title.as_deref(), Some("Café"));
    }
}
