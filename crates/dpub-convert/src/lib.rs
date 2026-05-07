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
use dpub_util::xml::{escape_attr, escape_text};
use epub3_writer::{
    AccessMode, AudioFile, ContentDocument, MediaOverlay, Nav, NavListItem, OverlayItem,
    OverlayPar, OverlaySeq, PackageMetadata, Publication, SectionPart,
};
use rayon::prelude::*;

mod error;
mod text_cleanup;
pub use error::{Error, Result};

/// Convert a parsed DAISY 2.02 [`Book`] into an EPUB 3 [`Publication`].
///
/// `book.root` is preserved as the source path for audio files; the
/// resulting [`Publication`] holds [`AudioFile::source_path`] entries
/// pointing at the originals on disk, which [`Publication::write_zip`]
/// streams in at write time.
pub fn convert(book: &Book) -> Result<Publication> {
    // Pre-bucket nav items by the SMIL filename they live in, so each section
    // can fetch its own nav entries in O(1) instead of scanning the full nav
    // list once per section. Without this, a 30-section / 364-nav-item book
    // does ~10K extra string comparisons during conversion.
    let nav_by_smil = bucket_nav_by_smil(book);

    let metadata = build_package_metadata(book);
    let nav = build_nav(book);
    let sections = build_sections(book, &nav_by_smil)?;
    let audio_files = build_audio_files(book);

    Ok(Publication {
        metadata,
        nav,
        sections,
        audio_files,
        cover: None,
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
        rights: m.other.get("dc:rights").cloned(),
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

fn build_sections(
    book: &Book,
    nav_by_smil: &std::collections::HashMap<&str, Vec<&NavItem>>,
) -> Result<Vec<SectionPart>> {
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

            let (body_xhtml, anchors) = build_section_body(book, idx, nav_by_smil);
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

/// One pass over `book.ncc.nav` building a SMIL filename → nav-items map,
/// so [`build_section_body`] can fetch its own entries in O(1) per section
/// instead of re-scanning the full nav list.
fn bucket_nav_by_smil(book: &Book) -> std::collections::HashMap<&str, Vec<&NavItem>> {
    let mut map: std::collections::HashMap<&str, Vec<&NavItem>> = std::collections::HashMap::new();
    for item in &book.ncc.nav {
        let href = match item {
            NavItem::Heading(h) => &h.href,
            NavItem::Page(p) => &p.href,
        };
        let smil_filename = href.split_once('#').map_or(href.as_str(), |(f, _)| f);
        map.entry(smil_filename).or_default().push(item);
    }
    map
}

/// Render the section's content document. We populate it with the section's
/// heading and any anchors referenced from SMIL `<text>` elements, so the
/// overlay's text references resolve to a real id in the document.
fn build_section_body(
    book: &Book,
    idx: usize,
    nav_by_smil: &std::collections::HashMap<&str, Vec<&NavItem>>,
) -> (String, Vec<String>) {
    let mut html = String::with_capacity(512);
    let smil_filename = book.master.references[idx].src.as_str();

    // O(1) lookup of the nav items that live in this SMIL file.
    let entries: &[&NavItem] = nav_by_smil.get(smil_filename).map_or(&[], Vec::as_slice);

    let mut anchors = Vec::with_capacity(entries.len());
    for &entry in entries {
        match entry {
            NavItem::Heading(h) => {
                let anchor = href_anchor(&h.href).unwrap_or_else(|| h.id.clone());
                let level = h.level.clamp(1, 6);
                let _ = std::fmt::Write::write_fmt(
                    &mut html,
                    format_args!(
                        "  <h{level} id=\"{anchor}\">{label}</h{level}>\n",
                        anchor = escape_attr(&anchor),
                        label = escape_text(&heading_label(h)),
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
                        anchor = escape_attr(&anchor),
                        label = escape_attr(&p.text),
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
                escape_text(&book.master.references[idx].title),
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

/// Audio handling for the converted publication.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Embed the source audio files unchanged. Default.
    #[default]
    Original,
    /// Re-encode every distinct source audio file to Ogg/Opus at the given
    /// bitrate (kbit/s). The Media Overlay timing references stay in
    /// seconds, so they continue to point at the same temporal location
    /// after re-encoding.
    Opus { bitrate_kbps: u32 },
}

/// Optional Whisper-driven transcription that fills the text layer of an
/// audio-only DAISY book. When set, every distinct audio file is decoded
/// and transcribed; the resulting segments get placed in the content
/// XHTMLs in time order.
#[derive(Debug, Clone)]
pub struct TranscribeOptions {
    /// Path to a `ggml-*.bin` Whisper model. See the `dpub-whisper` crate
    /// docs for download links.
    pub model_path: std::path::PathBuf,
    /// ISO 639-1 language code, e.g. `"nl"` for Dutch.
    pub language: String,
}

/// Knobs for [`convert_to_file`].
#[derive(Debug, Default, Clone)]
pub struct ConvertOptions {
    pub audio: AudioFormat,
    pub transcribe: Option<TranscribeOptions>,
    /// When `true`, transcribed segments are emitted one `<p>` per
    /// Whisper segment. Default `false` — segments are merged into
    /// prose-shaped paragraphs of ~3–6 sentences each.
    pub raw_transcript_segments: bool,
    /// Optional path to a JPEG or PNG cover image. Embedded as a
    /// manifest item with `properties="cover-image"`.
    pub cover: Option<std::path::PathBuf>,
    /// When `true`, attempt a best-effort cover lookup against
    /// Open Library using the book's title/author/identifier. A
    /// miss is silent (no cover embedded); a network failure is
    /// silent. Mutually exclusive with `cover` at the CLI layer.
    pub auto_cover: bool,
    /// Free-text rights statement to stamp into the EPUB's
    /// `<dc:rights>` field. When `None`, the source DAISY's
    /// `dc:rights` (if any) carries through; the CLI override
    /// is the simplest way to assert rights when the source
    /// doesn't carry one.
    pub rights: Option<String>,
    /// When `true`, skip per-word Media Overlay sync and fall back
    /// to per-paragraph sync. Default `false` — the cleanup path
    /// emits one SMIL `<par>` per word, anchored to per-word
    /// `<span>`s in the content XHTML, for karaoke-style
    /// highlight-along-with-audio. Set this to keep SMIL files
    /// small at the cost of a coarser reading experience.
    pub no_word_sync: bool,
}

/// Convert and write a DAISY 2.02 publication to an EPUB 3 file in one call.
///
/// Pass `ConvertOptions::default()` (or `Default::default()`) for the common
/// case of "embed source audio unchanged". Set `opts.audio = AudioFormat::Opus
/// { bitrate_kbps }` to re-encode every audio file to Ogg/Opus before writing
/// — this requires `ffmpeg` on PATH. Set `opts.transcribe = Some(...)` to
/// run local Whisper inference per audio file and populate the content
/// XHTMLs with the transcribed text.
pub fn convert_to_file(book: &Book, output: &Path, opts: &ConvertOptions) -> Result<()> {
    let mut publication = convert(book)?;

    // Kick off the cover lookup on a background thread so it overlaps
    // with transcription and audio recompression. The lookup only needs
    // metadata (available now) and the result is just bytes we slot in
    // before the ZIP write.
    let cover_handle = if opts.cover.is_some() {
        None // explicit cover — no network lookup needed
    } else if opts.auto_cover {
        let metadata = book.metadata().clone();
        Some(std::thread::spawn(move || auto_lookup_cover_from_meta(&metadata)))
    } else {
        None
    };

    if let Some(cover_path) = &opts.cover {
        publication.cover = Some(load_cover_image(cover_path)?);
    }

    if let Some(rights) = &opts.rights {
        publication.metadata.rights = Some(rights.clone());
    }

    // Transcribe BEFORE audio recompression — we want to feed Whisper the
    // original (typically MP3) bytes, not a lossy Opus pass that throws away
    // information whisper.cpp's frontend re-discards anyway.
    if let Some(transcribe) = &opts.transcribe {
        inject_transcripts(
            book,
            &mut publication,
            transcribe,
            opts.raw_transcript_segments,
            opts.no_word_sync,
        )?;
    }

    // Recompression has to happen *before* the ZIP write because the writer
    // streams audio bytes from `source_path` directly into the archive.
    let _scratch = match opts.audio {
        AudioFormat::Original => None,
        AudioFormat::Opus { bitrate_kbps } => {
            Some(recompress_audio_to_opus(&mut publication, bitrate_kbps)?)
        }
    };

    // Join the cover thread before writing the ZIP.
    if let Some(handle) = cover_handle {
        if let Ok(cover) = handle.join() {
            publication.cover = cover;
        }
    }

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

/// Read a JPEG or PNG file from disk and wrap it in a [`CoverImage`]
/// ready to embed in the manifest. Magic-byte sniffing only — no
/// decode, no resize. Anything else is rejected with
/// [`Error::UnsupportedCoverImage`].
fn load_cover_image(path: &Path) -> Result<epub3_writer::CoverImage> {
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let (media_type, ext) = sniff_image_format(&bytes).ok_or_else(|| {
        Error::UnsupportedCoverImage {
            path: path.to_path_buf(),
        }
    })?;
    Ok(epub3_writer::CoverImage {
        href: format!("images/cover.{ext}"),
        media_type: media_type.to_owned(),
        bytes,
    })
}

/// Best-effort cover lookup against Open Library. Takes owned
/// [`Metadata`] so it can run on a background thread. Logs a single
/// line describing the outcome — successful match, miss, or transport
/// error — and returns `None` for everything except a successful
/// match. Network errors are deliberately swallowed: `--auto-cover`
/// is best-effort and a missing cover is normal for older DAISY books.
fn auto_lookup_cover_from_meta(
    m: &dpub_core::Metadata,
) -> Option<epub3_writer::CoverImage> {
    let Some(title) = m.title.as_deref() else {
        eprintln!("--auto-cover: no title in metadata, skipping");
        return None;
    };
    let hints = dpub_meta::LookupHints {
        title,
        creator: m.creator.as_deref(),
        language: m.language.as_deref(),
        identifier: m.identifier.as_deref(),
    };
    match dpub_meta::lookup_cover(&hints) {
        Ok(Some(fetched)) => {
            eprintln!("--auto-cover: matched {}", fetched.provenance);
            let ext = if fetched.media_type == "image/png" { "png" } else { "jpg" };
            Some(epub3_writer::CoverImage {
                href: format!("images/cover.{ext}"),
                media_type: fetched.media_type,
                bytes: fetched.bytes,
            })
        }
        Ok(None) => {
            eprintln!("--auto-cover: no plausible match found, skipping");
            None
        }
        Err(e) => {
            eprintln!("--auto-cover: lookup failed ({e}), skipping");
            None
        }
    }
}

/// Returns `(media_type, file_extension)` if `bytes` start with a known
/// JPEG or PNG magic sequence. JPEG is `FF D8 FF`; PNG is the eight-byte
/// signature `89 50 4E 47 0D 0A 1A 0A`. Anything else returns `None`.
fn sniff_image_format(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(("image/png", "png"))
    } else {
        None
    }
}

/// For every section, find the audio files referenced by the section's
/// Media Overlay, transcribe each of them (caching across sections that
/// share an audio file), and append the time-ordered transcript as a list
/// of `<p>` paragraphs to the section's content XHTML.
///
/// Each paragraph gets a stable `id="tx-<section>-<para>"` so a future
/// per-paragraph Media Overlay sync milestone (M6.5) can reference them
/// without re-rendering the XHTML.
///
/// When `raw_segments` is `true`, the per-segment Whisper output is emitted
/// directly (one `<p>` per ~10–30 s segment); the default `false` runs
/// `text_cleanup::merge_into_paragraphs` to produce prose-shaped output.
fn inject_transcripts(
    book: &Book,
    publication: &mut Publication,
    opts: &TranscribeOptions,
    raw_segments: bool,
    no_word_sync: bool,
) -> Result<()> {
    let whisper_opts = dpub_whisper::TranscribeOptions {
        model_path: opts.model_path.clone(),
        language: opts.language.clone(),
    };
    // Load the GGML model exactly once for the whole book. Calling
    // `dpub_whisper::transcribe` per file would re-load 1.5 GB+ of
    // weights into Metal/CUDA buffers for every audio file (#10).
    let transcriber = dpub_whisper::Transcriber::new(&whisper_opts)?;

    // Cache: file basename → segments. Reused across sections that share an
    // audio file.
    let mut cache: std::collections::HashMap<String, Vec<dpub_whisper::Segment>> =
        std::collections::HashMap::new();

    for (idx, section_part) in publication.sections.iter_mut().enumerate() {
        // Collect the (audio basename, [t0, t1]) pairs this section uses,
        // in document order.
        let mut audio_ranges: Vec<(String, f64, f64)> = Vec::new();
        if let Some(section_smil) = book.sections.get(idx) {
            collect_audio_ranges(&section_smil.root, &mut audio_ranges);
        }

        // Transcribe the involved audio files (skipping any we already did).
        for (audio_basename, _, _) in &audio_ranges {
            if cache.contains_key(audio_basename) {
                continue;
            }
            let audio_full_path = book.root.join(audio_basename);
            let segments = transcriber.transcribe(&audio_full_path)?;
            cache.insert(audio_basename.clone(), segments);
        }

        // Collect the in-range segments in document order, paired with
        // the audio basename each came from so the cleanup state machine
        // can flush at audio-file boundaries.
        let mut section_segments: Vec<dpub_whisper::Segment> = Vec::new();
        let mut section_audio_srcs: Vec<String> = Vec::new();
        for (audio_basename, t0, t1) in &audio_ranges {
            let Some(segments) = cache.get(audio_basename) else {
                continue;
            };
            for seg in segments {
                let mid = (seg.start_seconds + seg.end_seconds) * 0.5;
                if mid >= *t0 && mid <= *t1 && !seg.text.is_empty() {
                    section_segments.push(seg.clone());
                    section_audio_srcs.push(audio_basename.clone());
                }
            }
        }

        let new_paragraphs = if raw_segments {
            render_raw_paragraphs(idx, &section_segments)
        } else {
            let cleaned = text_cleanup::merge_into_paragraphs(
                &section_segments,
                &section_audio_srcs,
                &text_cleanup::CleanupOpts::default(),
            );
            let html = render_cleaned_paragraphs(idx, &cleaned);
            // Word-level Media Overlay sync: rebuild this section's
            // overlay from the cleaned paragraphs, replacing the
            // heading-level shell that build_sections constructed.
            // The new overlay drives karaoke-style highlighting in
            // reading systems (Thorium etc.) that honour Media Overlays.
            if !no_word_sync
                && cleaned.iter().any(|p| !p.words.is_empty())
                && let Some(overlay) = section_part.overlay.as_mut()
            {
                let new_root = build_word_overlay_seq(
                    &section_part.content.href,
                    idx,
                    &cleaned,
                );
                overlay.root = new_root;
            }
            html
        };
        if !new_paragraphs.is_empty() {
            section_part.content.body_xhtml.push_str(&new_paragraphs);
        }
    }

    Ok(())
}

fn render_raw_paragraphs(section_idx: usize, segments: &[dpub_whisper::Segment]) -> String {
    let mut out = String::new();
    for (para_idx, seg) in segments.iter().enumerate() {
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                "  <p id=\"tx-{section_idx:03}-{para_idx:03}\">{}</p>\n",
                escape_text(&seg.text)
            ),
        );
    }
    out
}

/// Build a fresh `OverlaySeq` for a section from the cleaned paragraphs
/// produced by `text_cleanup::merge_into_paragraphs`.
///
/// Shape: an outer `<seq epub:textref="../content/sNN.xhtml">` wrapping
/// one inner `<seq epub:textref="...#tx-NNN-MMM">` per paragraph, each
/// wrapping one `<par id="w-NNN-MMM-KKK">` per word. This gives reading
/// systems (Thorium, Readium) a structural place to scope highlight to
/// "current paragraph" while still tracking the spoken word.
///
/// The `content_href` is EPUB-relative to the OPF (e.g.
/// `content/section-001.xhtml`); SMIL lives in `media-overlays/` so
/// every emitted `src` starts with `../`.
fn build_word_overlay_seq(
    content_href: &str,
    section_idx: usize,
    paragraphs: &[text_cleanup::Paragraph],
) -> OverlaySeq {
    let mut top_children: Vec<OverlayItem> = Vec::with_capacity(paragraphs.len());
    for (para_idx, para) in paragraphs.iter().enumerate() {
        if para.words.is_empty() {
            continue;
        }
        let para_anchor = format!("tx-{section_idx:03}-{para_idx:03}");
        let para_textref = format!("../{content_href}#{para_anchor}");
        let audio_src = format!("../audio/{}", para.audio_src);

        let mut word_children: Vec<OverlayItem> = Vec::with_capacity(para.words.len());
        for (word_idx, word) in para.words.iter().enumerate() {
            let word_id = format!("w-{section_idx:03}-{para_idx:03}-{word_idx:03}");
            word_children.push(OverlayItem::Par(OverlayPar {
                id: Some(word_id.clone()),
                text_src: format!("../{content_href}#{word_id}"),
                audio_src: audio_src.clone(),
                clip_begin_seconds: word.start_seconds,
                clip_end_seconds: word.end_seconds,
            }));
        }
        top_children.push(OverlayItem::Seq(OverlaySeq {
            textref: Some(para_textref),
            children: word_children,
        }));
    }
    OverlaySeq {
        textref: Some(format!("../{content_href}")),
        children: top_children,
    }
}

fn render_cleaned_paragraphs(
    section_idx: usize,
    paragraphs: &[text_cleanup::Paragraph],
) -> String {
    let mut out = String::new();
    for (para_idx, para) in paragraphs.iter().enumerate() {
        if para.words.is_empty() {
            // Whisper run with no per-word data (or test fixture
            // without words). Fall back to a flat <p> with the
            // paragraph text.
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "  <p id=\"tx-{section_idx:03}-{para_idx:03}\">{}</p>\n",
                    escape_text(&para.text)
                ),
            );
            continue;
        }
        // Emit one <span id="w-..."> per word, separated by single ASCII
        // spaces so reading systems render natural word spacing.
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("  <p id=\"tx-{section_idx:03}-{para_idx:03}\">"),
        );
        for (word_idx, word) in para.words.iter().enumerate() {
            if word_idx > 0 {
                out.push(' ');
            }
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "<span id=\"w-{section_idx:03}-{para_idx:03}-{word_idx:03}\">{}</span>",
                    escape_text(&word.text)
                ),
            );
        }
        out.push_str("</p>\n");
    }
    out
}

/// Walk a SectionSmil's `<seq>` tree collecting (audio basename, t0, t1)
/// triples for every `<par>` that has an associated audio span. The
/// audio basename is just the last path segment of the SMIL `audio src`
/// — that matches what `build_audio_files` puts into the EPUB.
fn collect_audio_ranges(seq: &SmilSeq, out: &mut Vec<(String, f64, f64)>) {
    for child in &seq.children {
        match child {
            SeqChild::Par(par) => {
                if let Some((src, t0, t1)) = par_audio_range(par) {
                    out.push((src, t0, t1));
                }
            }
            SeqChild::Seq(inner) => collect_audio_ranges(inner, out),
            SeqChild::Audio(_) => {
                // Bare audio at the seq root has no text-anchor companion,
                // so it doesn't get its own par; counted as part of an
                // enclosing par via collapse_par's logic.
            }
        }
    }
}

fn par_audio_range(par: &dpub_core::SmilPar) -> Option<(String, f64, f64)> {
    let (src, t0, t1) = collect_audio(par)?;
    Some((file_basename(&src), t0, t1))
}

/// Re-encode every audio file in the publication to Ogg/Opus, mutating the
/// publication's audio entries and overlay references in place.
///
/// Returns the [`tempfile::TempDir`] that holds the recompressed files.
/// The caller has to keep it alive until the publication has been written
/// (the audio bytes are streamed from disk during `write_zip`).
/// One audio file's planned destination inside the EPUB and on the
/// scratch filesystem during Opus re-encoding. Built up-front so the
/// parallel encoder phase can borrow `publication` immutably.
struct Plan {
    new_href: String,
    new_source: std::path::PathBuf,
}

fn recompress_audio_to_opus(
    publication: &mut Publication,
    bitrate_kbps: u32,
) -> Result<tempfile::TempDir> {
    let scratch = tempfile::tempdir().map_err(|source| Error::Io {
        path: std::env::temp_dir(),
        source,
    })?;

    let plans: Vec<Plan> = publication
        .audio_files
        .iter()
        .map(|audio| {
            let new_href = swap_extension(&audio.href, "opus");
            let new_source = scratch.path().join(
                std::path::Path::new(&new_href)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("audio.opus")),
            );
            Plan {
                new_href,
                new_source,
            }
        })
        .collect();

    // Run ffmpeg jobs in parallel. ffmpeg itself is multi-threaded inside one
    // file, but the spawn-and-wait round-trip per file is the dominant cost
    // for short audiobook chapters. rayon's default thread pool gives us
    // ~num_cpus parallelism for free, which on a 30-section / 8-core machine
    // typically takes a 6-minute encode down to ~1.5 minutes.
    publication
        .audio_files
        .par_iter()
        .zip(plans.par_iter())
        .try_for_each(|(audio, plan)| {
            dpub_audio::recompress_to_opus(&audio.source_path, &plan.new_source, bitrate_kbps)
                .map_err(Error::Audio)
        })?;

    // Apply the planned renames.
    for (audio, plan) in publication.audio_files.iter_mut().zip(plans) {
        audio.href = plan.new_href;
        audio.source_path = plan.new_source;
        audio.media_type = "audio/ogg; codecs=opus".into();
    }

    // Patch every Media Overlay's audio_src to point at the new file.
    // The overlay refs are paths relative to the SMIL location, of the form
    // "../audio/foo.mp3". Just strip the basename and rewrite the extension.
    for section in &mut publication.sections {
        if let Some(overlay) = section.overlay.as_mut() {
            rewrite_overlay_audio_refs(&mut overlay.root);
        }
    }

    Ok(scratch)
}

fn swap_extension(href: &str, new_ext: &str) -> String {
    let mut path = std::path::PathBuf::from(href);
    path.set_extension(new_ext);
    // PathBuf may use platform-specific separators; we want forward slashes
    // because `href` is a ZIP-internal path.
    path.to_string_lossy().replace('\\', "/")
}

fn rewrite_overlay_audio_refs(seq: &mut OverlaySeq) {
    for child in &mut seq.children {
        match child {
            OverlayItem::Par(par) => {
                par.audio_src = swap_extension(&par.audio_src, "opus");
            }
            OverlayItem::Seq(inner) => rewrite_overlay_audio_refs(inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_for_known_extensions() {
        assert_eq!(media_type_for("foo.mp3"), "audio/mpeg");
        assert_eq!(media_type_for("FOO.MP3"), "audio/mpeg"); // case-insensitive
        assert_eq!(media_type_for("foo.m4a"), "audio/mp4");
        assert_eq!(media_type_for("foo.mp4"), "audio/mp4");
        assert_eq!(media_type_for("foo.opus"), "audio/ogg; codecs=opus");
        assert_eq!(media_type_for("foo.ogg"), "audio/ogg; codecs=opus");
    }

    #[test]
    fn media_type_for_unknown_or_missing_extension() {
        assert_eq!(media_type_for("foo.flac"), "application/octet-stream");
        assert_eq!(media_type_for("foo"), "application/octet-stream"); // no ext
        assert_eq!(media_type_for(""), "application/octet-stream");
    }

    #[test]
    fn render_cleaned_paragraphs_emits_word_spans_when_words_present() {
        let para = text_cleanup::Paragraph {
            start_seconds: 0.0,
            end_seconds: 1.5,
            text: "Hallo wereld.".into(),
            words: vec![
                dpub_whisper::Word {
                    start_seconds: 0.0,
                    end_seconds: 0.5,
                    text: "Hallo".into(),
                },
                dpub_whisper::Word {
                    start_seconds: 0.5,
                    end_seconds: 1.5,
                    text: "wereld.".into(),
                },
            ],
            audio_src: "a.mp3".into(),
        };
        let html = render_cleaned_paragraphs(7, &[para]);
        assert!(
            html.contains(r#"<p id="tx-007-000">"#),
            "missing paragraph id: {html}"
        );
        assert!(
            html.contains(r#"<span id="w-007-000-000">Hallo</span>"#),
            "missing first word span: {html}"
        );
        assert!(
            html.contains(r#"<span id="w-007-000-001">wereld.</span>"#),
            "missing second word span: {html}"
        );
        // Single ASCII space between word spans.
        assert!(
            html.contains("</span> <span"),
            "missing inter-span space: {html}"
        );
    }

    #[test]
    fn render_cleaned_paragraphs_falls_back_to_flat_p_without_words() {
        // Defensive: when a Paragraph has no per-word data (test fixture
        // without words, or a Whisper run that returned empty word list)
        // the renderer must still emit something readable.
        let para = text_cleanup::Paragraph {
            start_seconds: 0.0,
            end_seconds: 1.0,
            text: "Hallo wereld.".into(),
            words: vec![],
            audio_src: "a.mp3".into(),
        };
        let html = render_cleaned_paragraphs(0, &[para]);
        assert!(
            html.contains(r#"<p id="tx-000-000">Hallo wereld.</p>"#),
            "expected flat <p> fallback, got: {html}"
        );
        assert!(!html.contains("<span"), "no spans in fallback: {html}");
    }

    #[test]
    fn build_word_overlay_seq_produces_nested_seqs_per_paragraph() {
        let para = text_cleanup::Paragraph {
            start_seconds: 0.0,
            end_seconds: 2.0,
            text: "Hallo wereld.".into(),
            words: vec![
                dpub_whisper::Word {
                    start_seconds: 0.0,
                    end_seconds: 0.5,
                    text: "Hallo".into(),
                },
                dpub_whisper::Word {
                    start_seconds: 0.5,
                    end_seconds: 2.0,
                    text: "wereld.".into(),
                },
            ],
            audio_src: "07_Inleiding.mp3".into(),
        };
        let root = build_word_overlay_seq("content/sec.xhtml", 7, &[para]);
        assert_eq!(root.textref.as_deref(), Some("../content/sec.xhtml"));
        assert_eq!(root.children.len(), 1);

        let OverlayItem::Seq(para_seq) = &root.children[0] else {
            panic!("expected paragraph-level <seq>, got {:?}", root.children[0]);
        };
        assert_eq!(
            para_seq.textref.as_deref(),
            Some("../content/sec.xhtml#tx-007-000")
        );
        assert_eq!(para_seq.children.len(), 2);

        let OverlayItem::Par(first_par) = &para_seq.children[0] else {
            panic!("expected per-word <par>, got {:?}", para_seq.children[0]);
        };
        assert_eq!(first_par.id.as_deref(), Some("w-007-000-000"));
        assert_eq!(first_par.text_src, "../content/sec.xhtml#w-007-000-000");
        assert_eq!(first_par.audio_src, "../audio/07_Inleiding.mp3");
        assert!((first_par.clip_begin_seconds - 0.0).abs() < 1e-9);
        assert!((first_par.clip_end_seconds - 0.5).abs() < 1e-9);

        let OverlayItem::Par(second_par) = &para_seq.children[1] else {
            panic!("expected per-word <par>, got {:?}", para_seq.children[1]);
        };
        assert_eq!(second_par.id.as_deref(), Some("w-007-000-001"));
        assert!((second_par.clip_end_seconds - 2.0).abs() < 1e-9);
    }

    #[test]
    fn build_word_overlay_seq_skips_paragraphs_without_words() {
        let p1 = text_cleanup::Paragraph {
            start_seconds: 0.0,
            end_seconds: 1.0,
            text: "no words here".into(),
            words: vec![],
            audio_src: "a.mp3".into(),
        };
        let p2 = text_cleanup::Paragraph {
            start_seconds: 1.0,
            end_seconds: 2.0,
            text: "Hi".into(),
            words: vec![dpub_whisper::Word {
                start_seconds: 1.0,
                end_seconds: 2.0,
                text: "Hi".into(),
            }],
            audio_src: "a.mp3".into(),
        };
        let root = build_word_overlay_seq("content/x.xhtml", 0, &[p1, p2]);
        // Paragraph 0 had no words and was skipped; paragraph 1 produced
        // one inner <seq>.
        assert_eq!(root.children.len(), 1);
        let OverlayItem::Seq(inner) = &root.children[0] else {
            panic!("expected one inner seq");
        };
        assert_eq!(
            inner.textref.as_deref(),
            Some("../content/x.xhtml#tx-000-001")
        );
    }

    #[test]
    fn render_cleaned_paragraphs_escapes_word_text() {
        let para = text_cleanup::Paragraph {
            start_seconds: 0.0,
            end_seconds: 0.5,
            text: r#"a<b&c"d"#.into(),
            words: vec![dpub_whisper::Word {
                start_seconds: 0.0,
                end_seconds: 0.5,
                text: r#"a<b&c"d"#.into(),
            }],
            audio_src: "a.mp3".into(),
        };
        let html = render_cleaned_paragraphs(0, &[para]);
        assert!(
            html.contains("a&lt;b&amp;c"),
            "expected XML escaping in word text: {html}"
        );
        assert!(!html.contains("a<b&c"), "raw < and & must not leak");
    }

    #[test]
    fn swap_extension_preserves_directory_with_forward_slashes() {
        assert_eq!(swap_extension("audio/foo.mp3", "opus"), "audio/foo.opus");
        assert_eq!(
            swap_extension("../audio/foo.mp3", "opus"),
            "../audio/foo.opus"
        );
        assert_eq!(swap_extension("foo", "opus"), "foo.opus"); // no original ext
    }

    #[test]
    fn smil_to_xhtml_filename_swaps_extension() {
        assert_eq!(smil_to_xhtml_filename("ptk000007.smil"), "ptk000007.xhtml");
        assert_eq!(smil_to_xhtml_filename("noext"), "noext.xhtml");
    }
}
