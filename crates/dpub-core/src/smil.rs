//! DAISY 2.02 SMIL 1.0 model + parser.
//!
//! DAISY 2.02 books contain two kinds of SMIL files:
//!
//! - **`master.smil`** is a top-level "spine" that references each per-section
//!   SMIL via `<ref src="ptkNNNNNN.smil">` elements.
//! - **Per-section SMIL** files (one per heading or chunk of audio) contain
//!   the actual time-synchronised structure: a root `<seq>` of `<par>`
//!   elements, where each `<par>` ties a `<text>` reference (back to the NCC)
//!   to one or more `<audio>` clips.

use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Error, Result};

/// A `master.smil` file: metadata + flat list of section references.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MasterSmil {
    pub metadata: SmilMetadata,
    pub references: Vec<SectionRef>,
}

/// One `<ref>` in a master.smil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRef {
    pub id: String,
    pub title: String,
    /// Filename of the section SMIL, relative to the publication root.
    pub src: String,
}

/// A per-section `ptkNNNNNN.smil`: metadata + a single root `<seq>`.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionSmil {
    pub metadata: SmilMetadata,
    pub root: SmilSeq,
}

/// SMIL `<head><meta>` collection. Order is preserved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SmilMetadata {
    pub entries: Vec<(String, String)>,
}

impl SmilMetadata {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// `<seq>` — a sequential container.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SmilSeq {
    pub id: Option<String>,
    /// Total `dur` attribute, in seconds (parsed from a SMIL clock value).
    pub dur: Option<f64>,
    pub children: Vec<SeqChild>,
}

/// Direct children of a `<seq>` we care about.
#[derive(Debug, Clone, PartialEq)]
pub enum SeqChild {
    Par(SmilPar),
    Seq(SmilSeq),
    /// Audio clips can also appear directly inside a `<seq>`, particularly the
    /// nested `<seq>` of audio fragments inside a DAISY 2.02 `<par>`.
    Audio(AudioClip),
}

/// `<par>` — a parallel container, classically holding one `<text>` plus a
/// nested `<seq>` of audio clips (the DAISY 2.02 audio-only convention).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SmilPar {
    pub id: Option<String>,
    pub endsync: Option<String>,
    pub children: Vec<ParChild>,
}

/// Children of a `<par>` we care about.
#[derive(Debug, Clone, PartialEq)]
pub enum ParChild {
    Text(TextRef),
    Audio(AudioClip),
    Seq(SmilSeq),
}

/// `<text src="...#anchor" id="..."/>` reference back to the NCC or content document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRef {
    pub id: Option<String>,
    pub src: String,
}

/// `<audio src="..." clip-begin="..." clip-end="..." id="..."/>` clip.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub id: Option<String>,
    pub src: String,
    pub clip_begin: f64,
    pub clip_end: f64,
}

impl AudioClip {
    pub fn duration(&self) -> f64 {
        (self.clip_end - self.clip_begin).max(0.0)
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl MasterSmil {
    pub fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_bytes(&bytes, path)
    }

    pub fn parse_bytes(bytes: &[u8], path: &Path) -> Result<Self> {
        let text = String::from_utf8_lossy(bytes);
        let mut reader = Reader::from_str(&text);
        reader.config_mut().trim_text(true);

        let mut master = MasterSmil::default();
        let mut buf = Vec::new();
        let mut in_head = false;

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
                    if name_of(&e) == "head" {
                        in_head = true;
                    }
                }
                Ok(Event::End(e)) => {
                    if name_of_end(&e) == "head" {
                        in_head = false;
                    }
                }
                Ok(Event::Empty(e)) => match name_of(&e) {
                    "meta" if in_head => {
                        if let Some((n, v)) = read_meta(&e) {
                            master.metadata.entries.push((n, v));
                        }
                    }
                    "ref" => {
                        master.references.push(SectionRef {
                            id: attr_str(&e, "id").unwrap_or_default(),
                            title: attr_str(&e, "title").unwrap_or_default(),
                            src: attr_str(&e, "src").unwrap_or_default(),
                        });
                    }
                    _ => {}
                },
                Ok(_) => {}
            }
            buf.clear();
        }

        Ok(master)
    }
}

impl SectionSmil {
    pub fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse_bytes(&bytes, path)
    }

    pub fn parse_bytes(bytes: &[u8], path: &Path) -> Result<Self> {
        let text = String::from_utf8_lossy(bytes);
        let mut reader = Reader::from_str(&text);
        reader.config_mut().trim_text(true);

        let mut metadata = SmilMetadata::default();
        let mut buf = Vec::new();
        let mut in_head = false;
        let mut in_body = false;

        // Stack-based parser: each entry is the container we're currently appending into.
        let mut stack: Vec<Frame> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Err(source) => {
                    return Err(Error::Xml {
                        path: path.to_path_buf(),
                        source,
                    });
                }
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => match name_of(&e) {
                    "head" => in_head = true,
                    "body" => in_body = true,
                    "seq" if in_body => {
                        let seq = SmilSeq {
                            id: attr_str(&e, "id"),
                            dur: attr_str(&e, "dur")
                                .and_then(|d| crate::time::parse_clock_value(&d).ok()),
                            children: Vec::new(),
                        };
                        stack.push(Frame::Seq(seq));
                    }
                    "par" if in_body => {
                        let par = SmilPar {
                            id: attr_str(&e, "id"),
                            endsync: attr_str(&e, "endsync"),
                            children: Vec::new(),
                        };
                        stack.push(Frame::Par(par));
                    }
                    _ => {}
                },
                Ok(Event::End(e)) => match name_of_end(&e) {
                    "head" => in_head = false,
                    "body" => in_body = false,
                    "seq" => {
                        if let Some(Frame::Seq(seq)) = stack.pop() {
                            if stack.is_empty() {
                                // Root <seq>: keep it on the stack to be picked
                                // up after the loop; nothing to attach to.
                                stack.push(Frame::Seq(seq));
                            } else {
                                push_to_parent(&mut stack, ContainerChild::Seq(seq));
                            }
                        }
                    }
                    "par" => {
                        if let Some(Frame::Par(par)) = stack.pop() {
                            if stack.is_empty() {
                                stack.push(Frame::Par(par));
                            } else {
                                push_to_parent(&mut stack, ContainerChild::Par(par));
                            }
                        }
                    }
                    _ => {}
                },
                Ok(Event::Empty(e)) => match name_of(&e) {
                    "meta" if in_head => {
                        if let Some((n, v)) = read_meta(&e) {
                            metadata.entries.push((n, v));
                        }
                    }
                    "text" if matches!(stack.last(), Some(Frame::Par(_))) => {
                        let t = TextRef {
                            id: attr_str(&e, "id"),
                            src: attr_str(&e, "src").unwrap_or_default(),
                        };
                        if let Some(Frame::Par(par)) = stack.last_mut() {
                            par.children.push(ParChild::Text(t));
                        }
                    }
                    "audio" => {
                        let clip_begin = attr_str(&e, "clip-begin")
                            .or_else(|| attr_str(&e, "clipBegin"))
                            .and_then(|v| crate::time::parse_clock_value(&v).ok())
                            .unwrap_or(0.0);
                        let clip_end = attr_str(&e, "clip-end")
                            .or_else(|| attr_str(&e, "clipEnd"))
                            .and_then(|v| crate::time::parse_clock_value(&v).ok())
                            .unwrap_or(0.0);
                        let clip = AudioClip {
                            id: attr_str(&e, "id"),
                            src: attr_str(&e, "src").unwrap_or_default(),
                            clip_begin,
                            clip_end,
                        };
                        match stack.last_mut() {
                            Some(Frame::Par(par)) => par.children.push(ParChild::Audio(clip)),
                            Some(Frame::Seq(seq)) => seq.children.push(SeqChild::Audio(clip)),
                            None => {}
                        }
                    }
                    _ => {}
                },
                Ok(_) => {}
            }
            buf.clear();
        }

        // Whatever is left on the stack is the root. The root is the outermost <seq>.
        let root = match stack.into_iter().next() {
            Some(Frame::Seq(seq)) => seq,
            _ => SmilSeq::default(),
        };

        Ok(SectionSmil { metadata, root })
    }
}

enum ContainerChild {
    Seq(SmilSeq),
    Par(SmilPar),
}

fn push_to_parent(stack: &mut [Frame], child: ContainerChild) {
    let Some(parent) = stack.last_mut() else {
        // Root — the popped child IS the root, caller handles via leftover stack.
        // We re-push via a workaround: insert it back at top-level.
        // (Handled in SectionSmil::parse_bytes via the leftover stack.)
        // To keep behaviour symmetric we do nothing here; the root <seq> never
        // gets popped because there's nothing above it.
        unreachable!("push_to_parent called with empty stack")
    };
    match (parent, child) {
        (Frame::Seq(parent_seq), ContainerChild::Seq(seq)) => {
            parent_seq.children.push(SeqChild::Seq(seq));
        }
        (Frame::Seq(parent_seq), ContainerChild::Par(par)) => {
            parent_seq.children.push(SeqChild::Par(par));
        }
        (Frame::Par(parent_par), ContainerChild::Seq(seq)) => {
            parent_par.children.push(ParChild::Seq(seq));
        }
        (Frame::Par(_), ContainerChild::Par(_)) => {
            // <par> inside <par> is not part of the DAISY 2.02 profile; ignore.
        }
    }
}

enum Frame {
    Seq(SmilSeq),
    Par(SmilPar),
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn name_of<'a>(e: &'a quick_xml::events::BytesStart<'a>) -> &'a str {
    std::str::from_utf8(e.name().0).unwrap_or("")
}

fn name_of_end<'a>(e: &'a quick_xml::events::BytesEnd<'a>) -> &'a str {
    std::str::from_utf8(e.name().0).unwrap_or("")
}

fn attr_str(e: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key.as_bytes())
        .map(|a| a.unescape_value().unwrap_or_default().into_owned())
}

fn read_meta(e: &quick_xml::events::BytesStart<'_>) -> Option<(String, String)> {
    let name = attr_str(e, "name")?;
    let content = attr_str(e, "content")?;
    Some((name, content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_master_smil() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<smil>
<head>
<meta name="dc:title" content="Test"/>
<meta name="ncc:totalTime" content="00:42:00"/>
</head>
<body>
<ref title="Title" src="ptk001.smil" id="h1"/>
<ref title="Inleiding" src="ptk002.smil" id="h2"/>
</body>
</smil>"#;
        let m = MasterSmil::parse_bytes(xml.as_bytes(), Path::new("master.smil")).unwrap();
        assert_eq!(m.references.len(), 2);
        assert_eq!(m.references[1].title, "Inleiding");
        assert_eq!(m.references[1].src, "ptk002.smil");
        assert_eq!(m.metadata.get("dc:title"), Some("Test"));
    }

    #[test]
    fn parse_section_smil() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<smil>
<head>
<meta name="dc:title" content="Inleiding"/>
<meta name="ncc:totalElapsedTime" content="00:00:00"/>
</head>
<body>
<seq dur="10.000s">
<par endsync="last">
<text src="ncc.html#h1" id="b1"/>
<seq>
<audio src="07.mp3" clip-begin="npt=0.000s" clip-end="npt=1.750s" id="a1"/>
<audio src="07.mp3" clip-begin="npt=1.750s" clip-end="npt=4.500s" id="a2"/>
</seq>
</par>
</seq>
</body>
</smil>"#;
        let s = SectionSmil::parse_bytes(xml.as_bytes(), Path::new("ptk001.smil")).unwrap();
        assert!((s.root.dur.unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(s.root.children.len(), 1);
        let SeqChild::Par(par) = &s.root.children[0] else {
            panic!("expected par");
        };
        assert_eq!(par.endsync.as_deref(), Some("last"));
        assert_eq!(par.children.len(), 2);
        let ParChild::Text(t) = &par.children[0] else {
            panic!("expected text");
        };
        assert_eq!(t.src, "ncc.html#h1");
        let ParChild::Seq(audio_seq) = &par.children[1] else {
            panic!("expected seq");
        };
        assert_eq!(audio_seq.children.len(), 2);
        let SeqChild::Audio(a) = &audio_seq.children[0] else {
            panic!("expected audio");
        };
        assert!((a.duration() - 1.75).abs() < 1e-9);
    }

    #[test]
    fn audio_only_par_without_text() {
        // Some DAISY 2.02 producers emit <par> with only <audio> (no <text>).
        let xml = r#"<?xml version="1.0"?>
<smil><body><seq>
<par><audio src="x.mp3" clip-begin="npt=0s" clip-end="npt=2s"/></par>
</seq></body></smil>"#;
        let s = SectionSmil::parse_bytes(xml.as_bytes(), Path::new("ptk.smil")).unwrap();
        let SeqChild::Par(par) = &s.root.children[0] else {
            panic!();
        };
        assert!(matches!(&par.children[0], ParChild::Audio(_)));
    }
}
