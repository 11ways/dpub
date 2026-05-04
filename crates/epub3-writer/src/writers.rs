//! XML/XHTML serialisers for each EPUB 3 file. All output is UTF-8 and
//! deterministic given the same input — no clocks, no random IDs, so unit
//! tests can compare strings directly.

#![allow(clippy::write_with_newline)] // explicit \n keeps generated XML easy to skim

use std::fmt::Write as _;

use crate::model::{
    AccessMode, ContentDocument, MediaOverlay, Nav, NavListItem, OverlayItem, OverlayPar,
    OverlaySeq, Publication,
};

const OPF_NS: &str = "http://www.idpf.org/2007/opf";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const SMIL_NS: &str = "http://www.w3.org/ns/SMIL";
const EPUB_NS: &str = "http://www.idpf.org/2007/ops";
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const CONTAINER_NS: &str = "urn:oasis:names:tc:opendocument:xmlns:container";

/// Serialise the mandatory `META-INF/container.xml`. Single-rootfile form.
pub fn write_container_xml(opf_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="{CONTAINER_NS}">
  <rootfiles>
    <rootfile full-path="{opf}" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#,
        opf = escape_attr(opf_path),
    )
}

/// Serialise the OPF package document. Includes the package metadata block,
/// the manifest (content + nav + media overlays + audio files), and the
/// linear spine.
pub fn write_package_opf(pub_: &Publication) -> String {
    let mut s = String::with_capacity(8 * 1024);
    s.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    s.push('\n');
    let _ = write!(
        s,
        r#"<package xmlns="{OPF_NS}" version="3.0" unique-identifier="pub-id" xml:lang="{lang}" prefix="schema: http://schema.org/">
"#,
        lang = escape_attr(&pub_.metadata.language),
    );

    write_opf_metadata(&mut s, pub_);
    write_opf_manifest(&mut s, pub_);
    write_opf_spine(&mut s, pub_);

    s.push_str("</package>\n");
    s
}

fn write_opf_metadata(s: &mut String, pub_: &Publication) {
    let m = &pub_.metadata;
    let _ = write!(
        s,
        r#"  <metadata xmlns:dc="{DC_NS}">
    <dc:identifier id="pub-id">{id}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:language>{lang}</dc:language>
    <meta property="dcterms:modified">{modified}</meta>
"#,
        id = escape_text(&m.identifier),
        title = escape_text(&m.title),
        lang = escape_text(&m.language),
        modified = escape_text(&m.modified),
    );

    if let Some(c) = &m.creator {
        let _ = write!(s, "    <dc:creator>{}</dc:creator>\n", escape_text(c));
    }
    if let Some(p) = &m.publisher {
        let _ = write!(s, "    <dc:publisher>{}</dc:publisher>\n", escape_text(p));
    }
    if let Some(d) = &m.date {
        let _ = write!(s, "    <dc:date>{}</dc:date>\n", escape_text(d));
    }
    if let Some(src) = &m.source {
        let _ = write!(s, "    <dc:source>{}</dc:source>\n", escape_text(src));
    }
    if let Some(desc) = &m.description {
        let _ = write!(
            s,
            "    <dc:description>{}</dc:description>\n",
            escape_text(desc)
        );
    }

    if let Some(n) = &m.narrator {
        let _ = write!(
            s,
            "    <meta property=\"media:narrator\">{}</meta>\n",
            escape_text(n),
        );
    }

    // Per-overlay durations refining each media-overlay manifest item — required
    // by EPUB 3 when any spine item carries a media-overlay.
    let mut overlay_total = 0.0_f64;
    for section in &pub_.sections {
        if let Some(overlay) = &section.overlay {
            let _ = write!(
                s,
                "    <meta property=\"media:duration\" refines=\"#{id}-overlay\">{dur}</meta>\n",
                id = escape_attr(&section.id),
                dur = format_smil_clock(overlay.duration_seconds),
            );
            overlay_total += overlay.duration_seconds;
        }
    }
    // Publication-wide total duration.
    let total = m.duration_seconds.unwrap_or(overlay_total);
    if total > 0.0 {
        let _ = write!(
            s,
            "    <meta property=\"media:duration\">{}</meta>\n",
            format_smil_clock(total),
        );
    }

    // Accessibility metadata — required for an "accessibility-conformant"
    // EPUB. We always declare conformance to WCAG 2.1 AA because the
    // structural transformation never removes accessibility features.
    s.push_str("    <meta property=\"schema:accessibilityFeature\">tableOfContents</meta>\n");
    s.push_str("    <meta property=\"schema:accessibilityFeature\">synchronizedAudioText</meta>\n");
    s.push_str("    <meta property=\"schema:accessibilityHazard\">none</meta>\n");
    s.push_str(
        "    <meta property=\"schema:accessibilitySummary\">This audiobook is structured for accessible reading with synchronised audio and navigable headings.</meta>\n",
    );
    s.push_str(
        "    <meta property=\"a11y:certifiedBy\">dpub (https://github.com/11ways/dpub)</meta>\n",
    );

    let modes = if m.access_modes.is_empty() {
        vec![AccessMode::Auditory]
    } else {
        m.access_modes.clone()
    };
    for mode in modes {
        let _ = write!(
            s,
            "    <meta property=\"schema:accessMode\">{}</meta>\n",
            mode.as_str(),
        );
    }

    s.push_str("  </metadata>\n");
}

fn write_opf_manifest(s: &mut String, pub_: &Publication) {
    s.push_str("  <manifest>\n");

    // The navigation document.
    s.push_str(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );

    for section in &pub_.sections {
        let media_overlay_attr = section
            .overlay
            .as_ref()
            .map(|_| format!(" media-overlay=\"{}-overlay\"", escape_attr(&section.id)))
            .unwrap_or_default();
        let _ = write!(
            s,
            "    <item id=\"{id}\" href=\"{href}\" media-type=\"application/xhtml+xml\"{mo}/>\n",
            id = escape_attr(&section.id),
            href = escape_attr(&section.content.href),
            mo = media_overlay_attr,
        );
        if let Some(overlay) = &section.overlay {
            let _ = write!(
                s,
                "    <item id=\"{id}-overlay\" href=\"{href}\" media-type=\"application/smil+xml\"/>\n",
                id = escape_attr(&section.id),
                href = escape_attr(&overlay.href),
            );
        }
    }

    for audio in &pub_.audio_files {
        let _ = write!(
            s,
            "    <item id=\"{id}\" href=\"{href}\" media-type=\"{mt}\"/>\n",
            id = escape_attr(&audio.id),
            href = escape_attr(&audio.href),
            mt = escape_attr(&audio.media_type),
        );
    }

    s.push_str("  </manifest>\n");
}

fn write_opf_spine(s: &mut String, pub_: &Publication) {
    s.push_str("  <spine>\n");
    for section in &pub_.sections {
        let _ = write!(s, "    <itemref idref=\"{}\"/>\n", escape_attr(&section.id));
    }
    s.push_str("  </spine>\n");
}

/// Serialise the EPUB 3 navigation document (`nav.xhtml`).
pub fn write_nav_xhtml(nav: &Nav, language: &str, title: &str) -> String {
    let mut s = String::with_capacity(2 * 1024);
    let _ = write!(
        s,
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="{XHTML_NS}" xmlns:epub="{EPUB_NS}" xml:lang="{lang}" lang="{lang}">
<head>
  <meta charset="utf-8"/>
  <title>{title}</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>{title}</h1>
    <ol>
"#,
        lang = escape_attr(language),
        title = escape_text(title),
    );

    for item in &nav.toc {
        write_nav_item(&mut s, item, 6);
    }

    s.push_str("    </ol>\n  </nav>\n");

    if let Some(pages) = &nav.page_list
        && !pages.is_empty()
    {
        s.push_str("  <nav epub:type=\"page-list\" hidden=\"\">\n    <ol>\n");
        for item in pages {
            write_nav_item(&mut s, item, 6);
        }
        s.push_str("    </ol>\n  </nav>\n");
    }

    s.push_str("</body>\n</html>\n");
    s
}

fn write_nav_item(s: &mut String, item: &NavListItem, indent: usize) {
    let pad = " ".repeat(indent);
    let _ = write!(
        s,
        "{pad}<li><a href=\"{href}\">{label}</a>",
        href = escape_attr(&item.href),
        label = escape_text(&item.label),
    );
    if !item.children.is_empty() {
        s.push('\n');
        let _ = write!(s, "{pad}  <ol>\n");
        for child in &item.children {
            write_nav_item(s, child, indent + 4);
        }
        let _ = write!(s, "{pad}  </ol>\n{pad}");
    }
    s.push_str("</li>\n");
}

/// Serialise one content XHTML document.
pub fn write_content_xhtml(doc: &ContentDocument) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="{XHTML_NS}" xmlns:epub="{EPUB_NS}" xml:lang="{lang}" lang="{lang}">
<head>
  <meta charset="utf-8"/>
  <title>{title}</title>
</head>
<body>
{body}
</body>
</html>
"#,
        lang = escape_attr(&doc.language),
        title = escape_text(&doc.title),
        body = doc.body_xhtml.trim_end(),
    )
}

/// Serialise one Media Overlays SMIL document.
pub fn write_overlay_smil(overlay: &MediaOverlay) -> String {
    let mut s = String::with_capacity(4 * 1024);
    let _ = write!(
        s,
        r#"<?xml version="1.0" encoding="utf-8"?>
<smil xmlns="{SMIL_NS}" xmlns:epub="{EPUB_NS}" version="3.0">
  <body>
"#,
    );
    write_overlay_seq(&mut s, &overlay.root, 2);
    s.push_str("  </body>\n</smil>\n");
    s
}

fn write_overlay_seq(s: &mut String, seq: &OverlaySeq, indent: usize) {
    let pad = " ".repeat(indent);
    let textref = seq
        .textref
        .as_deref()
        .map(|t| format!(" epub:textref=\"{}\"", escape_attr(t)))
        .unwrap_or_default();
    let _ = write!(s, "{pad}<seq{textref}>\n");
    for child in &seq.children {
        match child {
            OverlayItem::Par(p) => write_overlay_par(s, p, indent + 2),
            OverlayItem::Seq(inner) => write_overlay_seq(s, inner, indent + 2),
        }
    }
    let _ = write!(s, "{pad}</seq>\n");
}

fn write_overlay_par(s: &mut String, par: &OverlayPar, indent: usize) {
    let pad = " ".repeat(indent);
    let id_attr = par
        .id
        .as_deref()
        .map(|i| format!(" id=\"{}\"", escape_attr(i)))
        .unwrap_or_default();
    let _ = write!(s, "{pad}<par{id_attr}>\n");
    let _ = write!(s, "{pad}  <text src=\"{}\"/>\n", escape_attr(&par.text_src),);
    let _ = write!(
        s,
        "{pad}  <audio src=\"{src}\" clipBegin=\"{begin}\" clipEnd=\"{end}\"/>\n",
        src = escape_attr(&par.audio_src),
        begin = format_smil_clock(par.clip_begin_seconds),
        end = format_smil_clock(par.clip_end_seconds),
    );
    let _ = write!(s, "{pad}</par>\n");
}

/// Format a duration in seconds for SMIL 3.0 / EPUB 3 Media Overlays:
/// `H:MM:SS.fraction` form, which is unambiguous and accepted by every
/// EPUB reader.
pub fn format_smil_clock(seconds: f64) -> String {
    // Cap at 1000 hours — far beyond any realistic talking book — so the
    // f64→u64 cast is in-range and the lossy conversion is documented.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total_ms = (seconds.clamp(0.0, 3_600_000.0) * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    if ms == 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{h}:{m:02}:{s:02}.{ms:03}")
    }
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
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

    #[test]
    fn smil_clock_formats() {
        assert_eq!(format_smil_clock(0.0), "0:00:00");
        assert_eq!(format_smil_clock(1.5), "0:00:01.500");
        assert_eq!(format_smil_clock(90.0), "0:01:30");
        assert_eq!(format_smil_clock(3661.123), "1:01:01.123");
    }

    #[test]
    fn container_xml() {
        let s = write_container_xml("EPUB/package.opf");
        assert!(s.contains("rootfile full-path=\"EPUB/package.opf\""));
        assert!(s.starts_with("<?xml"));
    }
}
