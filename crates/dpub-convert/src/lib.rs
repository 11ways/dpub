//! DAISY 2.02 → EPUB 3 conversion.
//!
//! The transformation is structural and does not re-encode audio: source
//! MP3 files are embedded into the resulting EPUB byte-for-byte, with
//! `clipBegin` / `clipEnd` references rewritten into the SMIL 3.0 Media
//! Overlays form.
//!
//! Audio recompression is the job of M5 (`dpub-audio`); transcription of
//! audio-only books is M6 (`dpub-whisper`).

use std::path::Path;

use dpub_core::{Book, Heading, NavItem, ParChild, SeqChild, SmilSeq};
use epub3_writer::{
    AccessMode, AudioFile, ContentDocument, MediaOverlay, Nav, NavListItem, OverlayItem,
    OverlayPar, OverlaySeq, PackageMetadata, Publication, SectionPart,
};

mod error;
pub use error::{Error, Result};

/// Convert a parsed DAISY 2.02 [`Book`] into an EPUB 3 [`Publication`].
///
/// `book.root` is preserved as the source path for audio files; the
/// resulting [`Publication`] holds [`AudioFile::source_path`] entries
/// pointing at the originals on disk, which [`Publication::write_zip`]
/// streams in at write time.
pub fn convert(book: &Book) -> Result<Publication> {
    let metadata = build_package_metadata(book);
    let nav = build_nav(book);
    let sections = build_sections(book)?;
    let audio_files = build_audio_files(book);

    Ok(Publication {
        metadata,
        nav,
        sections,
        audio_files,
    })
}

fn build_package_metadata(book: &Book) -> PackageMetadata {
    let m = book.metadata();
    // DAISY identifiers are often bare integers (e.g. "5485"); wrap into a
    // URN form so the EPUB is globally unique even after the bare integer
    // collides with another publication.
    let identifier = m.identifier.as_deref().map_or_else(
        || format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        |raw| {
            if raw.starts_with("urn:") {
                raw.to_string()
            } else {
                format!("urn:dpub:daisy:{raw}")
            }
        },
    );

    let language = m
        .language
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("en")
        .to_owned();

    PackageMetadata {
        identifier,
        title: m.title.clone().unwrap_or_else(|| "Untitled".into()),
        language,
        modified: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        creator: m.creator.clone(),
        publisher: m.publisher.clone(),
        date: m.date.clone(),
        source: m
            .identifier
            .as_deref()
            .map(|raw| format!("urn:dpub:daisy:{raw}")),
        description: None,
        duration_seconds: Some(book.total_audio_seconds()),
        narrator: m.narrator.clone(),
        access_modes: if m.multimedia_type.as_deref() == Some("audioFullText") {
            vec![AccessMode::Auditory, AccessMode::Textual]
        } else {
            vec![AccessMode::Auditory]
        },
    }
}

fn build_nav(book: &Book) -> Nav {
    // Build a stack-based hierarchy: pop entries whose level is >= the
    // current heading's level, then push the new entry as a child of
    // whatever's left on top of the stack (or as a new top-level entry).
    //
    // A nav stack holds *raw* nodes and their level; we materialise the
    // tree from the stack when popping.
    let mut roots: Vec<NavListItem> = Vec::new();
    // Each entry: (level, accumulator). `accumulator` is the children of
    // the heading at that level (the heading itself sits one frame above).
    let mut stack: Vec<(u8, NavListItem)> = Vec::new();

    let close_down_to =
        |stack: &mut Vec<(u8, NavListItem)>, roots: &mut Vec<NavListItem>, level: u8| {
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                let (_, finished) = stack.pop().expect("non-empty");
                if let Some((_, parent)) = stack.last_mut() {
                    parent.children.push(finished);
                } else {
                    roots.push(finished);
                }
            }
        };

    for item in &book.ncc.nav {
        if let NavItem::Heading(h) = item {
            close_down_to(&mut stack, &mut roots, h.level);
            stack.push((
                h.level,
                NavListItem {
                    label: heading_label(h),
                    href: section_href_for_heading(h, book),
                    children: vec![],
                },
            ));
        }
    }
    close_down_to(&mut stack, &mut roots, 0);

    let pages: Vec<NavListItem> = book
        .ncc
        .pages()
        .map(|p| NavListItem {
            label: p.text.clone(),
            href: page_href(book, &p.href),
            children: vec![],
        })
        .collect();

    Nav {
        toc: roots,
        page_list: if pages.is_empty() { None } else { Some(pages) },
    }
}

fn heading_label(h: &Heading) -> String {
    if h.text.is_empty() {
        format!("Heading {}", h.id)
    } else {
        h.text.clone()
    }
}

fn section_href_for_heading(h: &Heading, _book: &Book) -> String {
    // Each DAISY section maps to one content XHTML named after its SMIL.
    // If the heading's href is "ptk000007.smil#bookid_000008", we point at
    // "content/ptk000007.xhtml#bookid_000008".
    let (file, anchor) = h.href.split_once('#').map_or_else(
        || (smil_to_xhtml_filename(&h.href), None),
        |(file, anchor)| (smil_to_xhtml_filename(file), Some(anchor.to_string())),
    );
    match anchor {
        Some(a) => format!("content/{file}#{a}"),
        None => format!("content/{file}"),
    }
}

fn page_href(_book: &Book, href: &str) -> String {
    let (file, anchor) = href.split_once('#').map_or_else(
        || (href.to_string(), None),
        |(f, a)| (f.to_string(), Some(a.to_string())),
    );
    let xhtml = smil_to_xhtml_filename(&file);
    match anchor {
        Some(a) => format!("content/{xhtml}#{a}"),
        None => format!("content/{xhtml}"),
    }
}

fn smil_to_xhtml_filename(smil: &str) -> String {
    smil.strip_suffix(".smil")
        .map_or_else(|| format!("{smil}.xhtml"), |s| format!("{s}.xhtml"))
}

fn build_sections(book: &Book) -> Result<Vec<SectionPart>> {
    book.master
        .references
        .iter()
        .zip(book.sections.iter())
        .enumerate()
        .map(|(idx, (section_ref, smil))| {
            let stem = section_ref
                .src
                .strip_suffix(".smil")
                .unwrap_or(&section_ref.src)
                .to_owned();
            let id = if stem.is_empty() {
                format!("section-{:03}", idx + 1)
            } else {
                stem.clone()
            };

            let content_href = format!("content/{stem}.xhtml");
            let overlay_href = format!("media-overlays/{stem}.smil");

            let (body_xhtml, anchors) = build_section_body(book, idx);
            let content = ContentDocument {
                href: content_href.clone(),
                title: section_ref.title.clone(),
                language: book
                    .metadata()
                    .language
                    .clone()
                    .unwrap_or_else(|| "en".into()),
                body_xhtml,
            };

            let duration = section_audio_seconds(&smil.root);
            let root = build_overlay_seq_from_section(book, idx, &content_href, &anchors);
            let overlay = MediaOverlay {
                href: overlay_href,
                duration_seconds: duration,
                root,
            };

            Ok(SectionPart {
                id,
                content,
                overlay: Some(overlay),
            })
        })
        .collect()
}

/// Render the section's content document. We populate it with the section's
/// heading and any anchors referenced from SMIL `<text>` elements, so the
/// overlay's text references resolve to a real id in the document.
fn build_section_body(book: &Book, idx: usize) -> (String, Vec<String>) {
    let mut html = String::with_capacity(512);
    let smil_filename = &book.master.references[idx].src;

    // Find headings/pages that point into this SMIL file, in document order.
    let entries: Vec<_> = book
        .ncc
        .nav
        .iter()
        .filter(|n| match n {
            NavItem::Heading(h) => h.href.starts_with(smil_filename.as_str()),
            NavItem::Page(p) => p.href.starts_with(smil_filename.as_str()),
        })
        .collect();

    let mut anchors = Vec::with_capacity(entries.len());
    for entry in &entries {
        match entry {
            NavItem::Heading(h) => {
                let anchor = href_anchor(&h.href).unwrap_or_else(|| h.id.clone());
                let level = h.level.clamp(1, 6);
                let _ = std::fmt::Write::write_fmt(
                    &mut html,
                    format_args!(
                        "  <h{level} id=\"{anchor}\">{label}</h{level}>\n",
                        anchor = escape(&anchor),
                        label = escape(&heading_label(h)),
                    ),
                );
                anchors.push(anchor);
            }
            NavItem::Page(p) => {
                let anchor = href_anchor(&p.href).unwrap_or_else(|| p.id.clone());
                let _ = std::fmt::Write::write_fmt(
                    &mut html,
                    format_args!(
                        "  <span epub:type=\"pagebreak\" id=\"{anchor}\" role=\"doc-pagebreak\" aria-label=\"{label}\"></span>\n",
                        anchor = escape(&anchor),
                        label = escape(&p.text),
                    ),
                );
                anchors.push(anchor);
            }
        }
    }

    if html.is_empty() {
        // Fallback: at least put the section title in the body so the file is
        // not totally empty (and stays valid XHTML body content).
        let _ = std::fmt::Write::write_fmt(
            &mut html,
            format_args!(
                "  <h1>{}</h1>\n",
                escape(&book.master.references[idx].title),
            ),
        );
    }

    (html, anchors)
}

fn href_anchor(href: &str) -> Option<String> {
    href.split_once('#').map(|(_, a)| a.to_string())
}

fn section_audio_seconds(seq: &SmilSeq) -> f64 {
    seq.children
        .iter()
        .map(|c| match c {
            SeqChild::Par(par) => par
                .children
                .iter()
                .map(|p| match p {
                    ParChild::Audio(a) => a.duration(),
                    ParChild::Seq(inner) => section_audio_seconds(inner),
                    ParChild::Text(_) => 0.0,
                })
                .sum::<f64>(),
            SeqChild::Seq(inner) => section_audio_seconds(inner),
            SeqChild::Audio(a) => a.duration(),
        })
        .sum()
}

/// Build the overlay's root `<seq>` for one section. Each DAISY `<par>`
/// becomes one EPUB MO `<par>` whose `<text src>` points back at the
/// section's content document and whose `<audio>` collapses any nested
/// audio sequence into a single `clipBegin`/`clipEnd` span.
fn build_overlay_seq_from_section(
    book: &Book,
    idx: usize,
    content_href: &str,
    section_anchors: &[String],
) -> OverlaySeq {
    let smil = &book.sections[idx];
    let mut anchors_iter = section_anchors.iter();
    let mut children = Vec::new();
    walk_root_seq(&smil.root, &mut anchors_iter, content_href, &mut children);

    OverlaySeq {
        // SMIL is in EPUB/media-overlays/, content doc in EPUB/content/, so
        // we step out of media-overlays/ and into content/.
        textref: Some(format!("../{content_href}")),
        children,
    }
}

fn walk_root_seq(
    seq: &SmilSeq,
    anchors: &mut std::slice::Iter<'_, String>,
    content_href: &str,
    out: &mut Vec<OverlayItem>,
) {
    for child in &seq.children {
        match child {
            SeqChild::Par(par) => {
                let anchor = anchors.next().cloned().unwrap_or_default();
                if let Some(par_overlay) = collapse_par(par, content_href, &anchor) {
                    out.push(OverlayItem::Par(par_overlay));
                }
            }
            SeqChild::Seq(inner) => walk_root_seq(inner, anchors, content_href, out),
            SeqChild::Audio(_) => {
                // Bare audio at the seq root has no text-anchor companion in
                // an audio-only DAISY layout; skip rather than produce a
                // text-less overlay par (which violates the MO spec).
            }
        }
    }
}

fn collapse_par(par: &dpub_core::SmilPar, content_href: &str, anchor: &str) -> Option<OverlayPar> {
    let (audio_src, begin, end) = collect_audio(par)?;
    let text_src = if anchor.is_empty() {
        format!("../{content_href}")
    } else {
        format!("../{content_href}#{anchor}")
    };
    Some(OverlayPar {
        id: par.id.clone(),
        text_src,
        audio_src: format!("../audio/{}", file_basename(&audio_src)),
        clip_begin_seconds: begin,
        clip_end_seconds: end,
    })
}

enum AudioVisit<'a> {
    Audio(&'a dpub_core::AudioClip),
    Seq(&'a SmilSeq),
    Par(&'a dpub_core::SmilPar),
}

fn visit_audio<'a>(
    children: impl IntoIterator<Item = AudioVisit<'a>>,
    src: &mut Option<String>,
    min_begin: &mut f64,
    max_end: &mut f64,
) {
    for v in children {
        match v {
            AudioVisit::Audio(a) => {
                if src.is_none() {
                    *src = Some(a.src.clone());
                }
                if Some(&a.src) == src.as_ref() {
                    *min_begin = min_begin.min(a.clip_begin);
                    *max_end = max_end.max(a.clip_end);
                }
            }
            AudioVisit::Seq(inner) => {
                let kids: Vec<AudioVisit> = inner
                    .children
                    .iter()
                    .map(|c| match c {
                        SeqChild::Audio(a) => AudioVisit::Audio(a),
                        SeqChild::Seq(s) => AudioVisit::Seq(s),
                        SeqChild::Par(p) => AudioVisit::Par(p),
                    })
                    .collect();
                visit_audio(kids, src, min_begin, max_end);
            }
            AudioVisit::Par(par) => {
                let kids: Vec<AudioVisit> = par
                    .children
                    .iter()
                    .filter_map(|c| match c {
                        ParChild::Audio(a) => Some(AudioVisit::Audio(a)),
                        ParChild::Seq(s) => Some(AudioVisit::Seq(s)),
                        ParChild::Text(_) => None,
                    })
                    .collect();
                visit_audio(kids, src, min_begin, max_end);
            }
        }
    }
}

fn collect_audio(par: &dpub_core::SmilPar) -> Option<(String, f64, f64)> {
    // Walk the par's children depth-first looking for audio clips that all
    // share a single source. Collapse them to one [begin..end] span.
    let mut src: Option<String> = None;
    let mut min_begin = f64::INFINITY;
    let mut max_end = f64::NEG_INFINITY;

    let initial: Vec<AudioVisit> = par
        .children
        .iter()
        .filter_map(|c| match c {
            ParChild::Audio(a) => Some(AudioVisit::Audio(a)),
            ParChild::Seq(s) => Some(AudioVisit::Seq(s)),
            ParChild::Text(_) => None,
        })
        .collect();
    visit_audio(initial, &mut src, &mut min_begin, &mut max_end);

    let src = src?;
    if !min_begin.is_finite() || !max_end.is_finite() {
        return None;
    }
    Some((src, min_begin, max_end))
}

fn build_audio_files(book: &Book) -> Vec<AudioFile> {
    book.audio_files()
        .into_iter()
        .enumerate()
        .map(|(i, src_name)| AudioFile {
            id: format!("audio-{:03}", i + 1),
            href: format!("audio/{}", file_basename(&src_name)),
            source_path: book.root.join(&src_name),
            media_type: media_type_for(&src_name),
        })
        .collect()
}

fn file_basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_owned()
}

fn media_type_for(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("mp3") => "audio/mpeg".into(),
        Some("m4a" | "mp4") => "audio/mp4".into(),
        Some("opus" | "ogg") => "audio/ogg; codecs=opus".into(),
        _ => "application/octet-stream".into(),
    }
}

fn escape(s: &str) -> String {
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

/// Convert and write a DAISY 2.02 publication to an EPUB 3 file in one call.
pub fn convert_to_file(book: &Book, output: &Path) -> Result<()> {
    let publication = convert(book)?;
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut f = std::fs::File::create(output).map_err(|source| Error::Io {
        path: output.to_path_buf(),
        source,
    })?;
    publication.write_zip(&mut f)?;
    Ok(())
}
