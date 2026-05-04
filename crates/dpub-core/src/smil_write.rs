//! Canonical-form writer for the SMIL data model.
//!
//! The output is *structurally* equivalent to the input — re-parsing it must
//! yield the same in-memory tree — but is **not** byte-stable against any
//! particular DAISY producer's output. Whitespace, attribute order, and the
//! XML prolog form are dictated by this writer, not by the source.
//!
//! For audio-only DAISY 2.02 (the common case), the canonical form re-uses
//! the SMIL 1.0 attribute spelling (`clip-begin` / `clip-end`) and the
//! `npt=` clock-value form, both of which `dpub-core` parses on input.

use std::fmt::Write as _;

use crate::smil::{
    AudioClip, MasterSmil, ParChild, SectionRef, SectionSmil, SeqChild, SmilMetadata, SmilPar,
    SmilSeq, TextRef,
};
use crate::time::format_clock_value;

const INDENT: &str = "  ";

/// Serialise a [`MasterSmil`] to a UTF-8 XML string.
pub fn write_master_smil(master: &MasterSmil) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    out.push('\n');
    out.push_str("<smil>\n");
    write_metadata(&mut out, &master.metadata, 1);
    out.push_str("<body>\n");
    for r in &master.references {
        write_section_ref(&mut out, r);
    }
    out.push_str("</body>\n");
    out.push_str("</smil>\n");
    out
}

/// Serialise a [`SectionSmil`] to a UTF-8 XML string.
pub fn write_section_smil(section: &SectionSmil) -> String {
    let mut out = String::with_capacity(8 * 1024);
    out.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    out.push('\n');
    out.push_str("<smil>\n");
    write_metadata(&mut out, &section.metadata, 1);
    out.push_str("<body>\n");
    write_seq(&mut out, &section.root, 1);
    out.push_str("</body>\n");
    out.push_str("</smil>\n");
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
}

fn write_metadata(out: &mut String, md: &SmilMetadata, level: usize) {
    if md.entries.is_empty() {
        return;
    }
    indent(out, level);
    out.push_str("<head>\n");
    for (name, content) in &md.entries {
        indent(out, level + 1);
        let _ = write!(
            out,
            r#"<meta name="{}" content="{}"/>"#,
            escape_attr(name),
            escape_attr(content),
        );
        out.push('\n');
    }
    indent(out, level);
    out.push_str("</head>\n");
}

fn write_section_ref(out: &mut String, r: &SectionRef) {
    indent(out, 1);
    let _ = write!(
        out,
        r#"<ref title="{}" src="{}" id="{}"/>"#,
        escape_attr(&r.title),
        escape_attr(&r.src),
        escape_attr(&r.id),
    );
    out.push('\n');
}

fn write_seq(out: &mut String, seq: &SmilSeq, level: usize) {
    indent(out, level);
    out.push_str("<seq");
    if let Some(id) = &seq.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    if let Some(d) = seq.dur {
        let _ = write!(out, r#" dur="{}""#, format_clock_value(d));
    }
    out.push_str(">\n");
    for child in &seq.children {
        match child {
            SeqChild::Par(p) => write_par(out, p, level + 1),
            SeqChild::Seq(s) => write_seq(out, s, level + 1),
            SeqChild::Audio(a) => write_audio(out, a, level + 1),
        }
    }
    indent(out, level);
    out.push_str("</seq>\n");
}

fn write_par(out: &mut String, par: &SmilPar, level: usize) {
    indent(out, level);
    out.push_str("<par");
    if let Some(id) = &par.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    if let Some(es) = &par.endsync {
        let _ = write!(out, r#" endsync="{}""#, escape_attr(es));
    }
    out.push_str(">\n");
    for child in &par.children {
        match child {
            ParChild::Text(t) => write_text(out, t, level + 1),
            ParChild::Audio(a) => write_audio(out, a, level + 1),
            ParChild::Seq(s) => write_seq(out, s, level + 1),
        }
    }
    indent(out, level);
    out.push_str("</par>\n");
}

fn write_text(out: &mut String, t: &TextRef, level: usize) {
    indent(out, level);
    let _ = write!(out, r#"<text src="{}""#, escape_attr(&t.src));
    if let Some(id) = &t.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    out.push_str("/>\n");
}

fn write_audio(out: &mut String, a: &AudioClip, level: usize) {
    indent(out, level);
    let _ = write!(
        out,
        r#"<audio src="{}" clip-begin="{}" clip-end="{}""#,
        escape_attr(&a.src),
        format_clock_value(a.clip_begin),
        format_clock_value(a.clip_end),
    );
    if let Some(id) = &a.id {
        let _ = write!(out, r#" id="{}""#, escape_attr(id));
    }
    out.push_str("/>\n");
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smil::{MasterSmil, SectionSmil};
    use std::path::Path;

    #[test]
    fn round_trip_master_synthetic() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<smil>
<head>
<meta name="dc:title" content="X"/>
<meta name="ncc:totalTime" content="00:01:00"/>
</head>
<body>
<ref title="A" src="ptk001.smil" id="h1"/>
<ref title="B" src="ptk002.smil" id="h2"/>
</body>
</smil>"#;
        let m1 = MasterSmil::parse_bytes(xml.as_bytes(), Path::new("master.smil")).unwrap();
        let serialised = write_master_smil(&m1);
        let m2 = MasterSmil::parse_bytes(serialised.as_bytes(), Path::new("master.smil")).unwrap();
        assert_eq!(m1, m2, "round trip changed AST\n{serialised}");
    }

    #[test]
    fn round_trip_section_synthetic() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<smil>
<head>
<meta name="dc:title" content="Test"/>
</head>
<body>
<seq dur="3.500s">
<par id="p1" endsync="last">
<text src="ncc.html#h1" id="t1"/>
<seq>
<audio src="x.mp3" clip-begin="npt=0.000s" clip-end="npt=1.500s" id="a1"/>
<audio src="x.mp3" clip-begin="npt=1.500s" clip-end="npt=3.500s" id="a2"/>
</seq>
</par>
</seq>
</body>
</smil>"#;
        let s1 = SectionSmil::parse_bytes(xml.as_bytes(), Path::new("ptk.smil")).unwrap();
        let serialised = write_section_smil(&s1);
        let s2 = SectionSmil::parse_bytes(serialised.as_bytes(), Path::new("ptk.smil")).unwrap();
        assert_eq!(s1, s2, "round trip changed AST\n{serialised}");
    }
}
